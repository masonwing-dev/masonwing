//! Per-tenant outbox relay loop.
//!
//! The relay consumes committed `run.start` outbox events, re-verifies
//! authority on PostgreSQL rows (never trusts the event), starts the durable
//! workflow under its stable identity, and only then marks the run STARTED.
//! A dispatch adapter that is not qualified leaves events to retry with
//! backoff — dispatch_state is never marked STARTED without durable
//! acceptance evidence from the engine.

use std::{sync::Arc, time::Duration};

use masonwing_contracts::Environment;
use masonwing_data_postgres::PostgresStore;
use masonwing_kernel::runtime::{ArtifactObjects, CommandFailure};
#[cfg(feature = "local-temporal-tests")]
use masonwing_workflow_temporal::LocalTemporalClient;
use uuid::Uuid;

/// Immutable, unavailable object store: the relay path never writes or reads
/// artifact bytes, and refusing all access keeps that true under fault.
struct UnavailableObjects;

#[async_trait::async_trait]
impl ArtifactObjects for UnavailableObjects {
    async fn put_immutable(
        &self,
        _key: &str,
        _bytes: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_STORE_UNAVAILABLE"))
    }

    async fn get_bounded(&self, _key: &str, _max_bytes: usize) -> Result<Vec<u8>, CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_STORE_UNAVAILABLE"))
    }
}

#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub database_url: String,
    pub tenants: Vec<Uuid>,
    pub temporal_address: String,
    pub temporal_namespace: String,
    pub task_queue: String,
    pub poll_interval: Duration,
    pub batch_size: i64,
    pub lease_seconds: i64,
    pub max_attempts: i32,
}

impl RelayConfig {
    /// Configuration only ever comes from operator environment; there is no
    /// code path where a plugin or tenant supplies it.
    pub fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing".into()
        });
        let tenants = std::env::var("MASONWING_WORKER_TENANTS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|t| Uuid::parse_str(t.trim()).ok())
            .collect();
        let temporal_address =
            std::env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| "http://127.0.0.1:39854".into());
        let temporal_namespace =
            std::env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| "masonwing-local".into());
        let task_queue = std::env::var("MASONWING_WORKER_TASK_QUEUE")
            .unwrap_or_else(|_| "masonwing-runs".into());
        let poll_ms: u64 = std::env::var("MASONWING_WORKER_POLL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        Ok(Self {
            database_url,
            tenants,
            temporal_address,
            temporal_namespace,
            task_queue,
            poll_interval: Duration::from_millis(poll_ms),
            batch_size: 16,
            lease_seconds: 30,
            max_attempts: 10,
        })
    }
}

/// One relay pass over all configured tenants. Returns how many events were
/// dispatched. Exposed for tests and for run-once operation.
pub async fn relay_once(store: &PostgresStore, config: &RelayConfig) -> usize {
    let mut dispatched = 0;
    for tenant in &config.tenants {
        let claimed = match store
            .claim_pending_outbox(*tenant, config.batch_size, config.lease_seconds)
            .await
        {
            Ok(claimed) => claimed,
            Err(_) => continue,
        };
        for event in claimed {
            if event.event_name != "run.start" || event.aggregate_type != "Run" {
                // Other aggregates have their own consumers; this relay does
                // not lease or acknowledge them — the claim query is narrowed
                // to run.start events so they are never picked up here.
                continue;
            }
            match process_run_event(store, config, *tenant, event.aggregate_id).await {
                Ok(true) => {
                    let _ = store.mark_outbox_delivered(*tenant, event.event_id).await;
                    dispatched += 1;
                }
                Ok(false) => {
                    // Authority already fenced the run; the event has nothing
                    // left to drive, so acknowledge it to avoid re-polling.
                    let _ = store.mark_outbox_delivered(*tenant, event.event_id).await;
                }
                Err(_) => {
                    let _ = store
                        .mark_outbox_failed(*tenant, event.event_id, config.max_attempts)
                        .await;
                }
            }
        }
    }
    dispatched
}

/// Start the durable workflow for a claimed run and return the engine's
/// execution identity.
///
/// Without the `local-temporal-tests` feature the durable-engine transport is
/// not compiled in at all, so dispatch fails closed: the event stays pending
/// and the run stays QUEUED rather than being marked STARTED on a guess.
#[cfg(feature = "local-temporal-tests")]
async fn start_durable_run(
    config: &RelayConfig,
    run: &masonwing_data_postgres::DispatchableRun,
    tenant: Uuid,
) -> Result<String, CommandFailure> {
    let temporal = LocalTemporalClient::connect(
        &config.temporal_address,
        &config.temporal_namespace,
        &config.task_queue,
    )
    .await
    .map_err(|_| CommandFailure::unavailable("DURABLE_ENGINE_UNAVAILABLE"))?;
    temporal
        .start_run_workflow(
            &run.temporal_workflow_id,
            &tenant.to_string(),
            &run.run_id.to_string(),
        )
        .await
        .map_err(|_| CommandFailure::unavailable("DURABLE_ENGINE_UNAVAILABLE"))
}

#[cfg(not(feature = "local-temporal-tests"))]
async fn start_durable_run(
    _config: &RelayConfig,
    _run: &masonwing_data_postgres::DispatchableRun,
    _tenant: Uuid,
) -> Result<String, CommandFailure> {
    Err(CommandFailure::unavailable("DURABLE_ENGINE_UNAVAILABLE"))
}

/// Verify a run's authority and start it on the durable engine. `Ok(true)`
/// means the workflow start was durably acknowledged and the run marked
/// STARTED; `Ok(false)` means the run was fenced (nothing to dispatch).
async fn process_run_event(
    store: &PostgresStore,
    config: &RelayConfig,
    tenant: Uuid,
    run_id: Uuid,
) -> Result<bool, CommandFailure> {
    let Some(run) = store.verify_and_claim_run(tenant, run_id).await? else {
        // Either already handled by another worker or fenced as BLOCKED.
        return Ok(false);
    };

    // The Temporal adapter is the only durable-engine transport, and it must
    // acknowledge by its stable identity before the run is ever marked STARTED.
    let temporal_run_id = start_durable_run(config, &run, tenant).await?;
    if temporal_run_id.is_empty() {
        // No durable acceptance evidence: leave the run QUEUED for retry
        // rather than inventing an identity.
        return Err(CommandFailure::unavailable("DURABLE_ENGINE_UNAVAILABLE"));
    }

    store
        .mark_run_started(tenant, run.run_id, &temporal_run_id, run.fence)
        .await?;
    Ok(true)
}

/// Run the relay loop until the process is asked to stop.
pub async fn run_relay(config: RelayConfig, environment: Environment) -> Result<(), String> {
    if matches!(
        environment,
        Environment::Production | Environment::StagingLive
    ) {
        return Err("unqualified relay cannot start in a live environment".into());
    }
    let objects: Arc<dyn ArtifactObjects> = Arc::new(UnavailableObjects);
    let store = PostgresStore::connect(&config.database_url, objects)
        .await
        .map_err(|e| format!("DATABASE_CONNECT_FAILED:{e}"))?;
    loop {
        if config.tenants.is_empty() {
            // In local development, idle instead of crashing when no static tenants configured
            tokio::time::sleep(config.poll_interval).await;
            continue;
        }
        let _ = relay_once(&store, &config).await;
        tokio::time::sleep(config.poll_interval).await;
    }
}
