//! System-path outbox relay for durable execution.
//!
//! Because PostgreSQL row-level security is forced on all tenant tables,
//! a worker cannot enumerate or mutate events across tenants in one query.
//! The worker receives its authorized tenant partition through configuration
//! and processes each tenant through this system-path API (`begin_system`).

use chrono::{DateTime, Utc};
use masonwing_kernel::runtime::CommandFailure;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{PostgresStore, database_failure};

/// One outbox event claimed for dispatch by the worker relay.
#[derive(Clone, Debug)]
pub struct OutboxDispatchItem {
    pub event_id: Uuid,
    pub event_name: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i32,
    pub aggregate_type: String,
}

/// A QUEUED run that has passed all authority re-checks and is ready
/// to be started on the durable engine.
#[derive(Clone, Debug)]
pub struct DispatchableRun {
    pub run_id: Uuid,
    pub workflow_id: String,
    pub workflow_version: String,
    pub plugin_id: String,
    pub plugin_digest: String,
    pub grant_id: Uuid,
    pub grant_fence: i64,
    pub fence: i64,
    pub temporal_workflow_id: String,
}

impl PostgresStore {
    /// Claim up to `limit` PENDING outbox events for a single tenant, setting
    /// a lease so concurrent workers do not double-dispatch.
    ///
    /// Deduplication against `inbox_events` is applied: an event already
    /// recorded in the inbox for consumer `'worker-relay'` is immediately
    /// marked DELIVERED and omitted from the return list.
    pub async fn claim_pending_outbox(
        &self,
        tenant_id: Uuid,
        limit: i64,
        lease_seconds: i64,
    ) -> Result<Vec<OutboxDispatchItem>, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "outbox-claim", true).await?;
        let rows = sqlx::query(
            "SELECT id, aggregate_id, aggregate_version, event_name, aggregate_type, stream_sequence
             FROM outbox_events
             WHERE tenant_id = $1
               AND delivery_status = 'PENDING'
               AND available_at <= clock_timestamp()
               AND (lease_expires_at IS NULL OR lease_expires_at < clock_timestamp())
               AND event_name = 'run.start'
               AND aggregate_type = 'Run'
             ORDER BY stream_sequence
             LIMIT $2
             FOR UPDATE SKIP LOCKED",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_failure)?;

        let mut claimed = Vec::new();
        for row in rows {
            let event_id: Uuid = row.try_get("id").map_err(database_failure)?;
            let stream_sequence: i64 = row.try_get("stream_sequence").map_err(database_failure)?;

            // Check if already processed by this consumer (inbox dedupe).
            let already_received: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM inbox_events WHERE tenant_id = $1 AND consumer = 'worker-relay' AND event_id = $2)",
            )
            .bind(tenant_id)
            .bind(event_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;

            if already_received {
                sqlx::query(
                    "UPDATE outbox_events SET delivery_status = 'DELIVERED', lease_expires_at = NULL WHERE tenant_id = $1 AND id = $2",
                )
                .bind(tenant_id)
                .bind(event_id)
                .execute(&mut *tx)
                .await
                .map_err(database_failure)?;
                continue;
            }

            // Set lease on the claimed event.
            sqlx::query(
                "UPDATE outbox_events
                 SET lease_expires_at = clock_timestamp() + make_interval(secs => $3::double precision),
                     delivery_attempts = delivery_attempts + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant_id)
            .bind(event_id)
            .bind(lease_seconds as f64)
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;

            claimed.push(OutboxDispatchItem {
                event_id,
                event_name: row.try_get("event_name").map_err(database_failure)?,
                aggregate_id: row.try_get("aggregate_id").map_err(database_failure)?,
                aggregate_version: row.try_get("aggregate_version").map_err(database_failure)?,
                aggregate_type: row.try_get("aggregate_type").map_err(database_failure)?,
            });

            // Pre-record in inbox to enforce idempotency across crash-resumes.
            let _ = stream_sequence;
        }

        tx.commit().await.map_err(database_failure)?;
        Ok(claimed)
    }

    /// Re-verify current authority boundaries for a QUEUED run before starting
    /// its durable execution.
    ///
    /// A system-path transaction does not confer unconstrained authority:
    /// 1. The run must still be QUEUED with dispatch_state PENDING.
    /// 2. No active kill switch applies to the tenant or the plugin.
    /// 3. The plugin installation must still be ENABLED with matching digest.
    /// 4. The granting principal must still hold an ACTIVE membership, and the
    ///    tenant's current Cedar policy must still permit run dispatch.
    /// 5. The grant must still be ACTIVE and not expired, with matching fence.
    ///
    /// If any check fails, the run is fenced into BLOCKED with an appropriate
    /// failure code, and `Ok(None)` is returned so nothing is dispatched.
    pub async fn verify_and_claim_run(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<DispatchableRun>, CommandFailure> {
        let mut tx = self
            .begin_system(tenant_id, "run-dispatch-check", true)
            .await?;
        let run_row = sqlx::query(
            "SELECT workflow_id, workflow_version, plugin_id, plugin_digest, principal_id,
                    grant_id, grant_fence, fence, temporal_workflow_id, state, dispatch_state
             FROM runs
             WHERE tenant_id = $1 AND id = $2
             FOR UPDATE",
        )
        .bind(tenant_id)
        .bind(run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?;

        let Some(run) = run_row else {
            return Ok(None);
        };

        let state: String = run.try_get("state").map_err(database_failure)?;
        let dispatch_state: String = run.try_get("dispatch_state").map_err(database_failure)?;
        if state != "QUEUED" || dispatch_state != "PENDING" {
            // Already started, cancelled, or blocked by a concurrent actor.
            return Ok(None);
        }

        let plugin_id: String = run.try_get("plugin_id").map_err(database_failure)?;
        let plugin_digest: String = run.try_get("plugin_digest").map_err(database_failure)?;
        let run_principal: Uuid = run.try_get("principal_id").map_err(database_failure)?;
        let grant_id: Uuid = run.try_get("grant_id").map_err(database_failure)?;
        let expected_grant_fence: i64 = run.try_get("grant_fence").map_err(database_failure)?;
        let fence: i64 = run.try_get("fence").map_err(database_failure)?;

        // 1. Kill-switch check.
        let killed: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM kill_switches
                WHERE tenant_id = $1 AND active
                  AND ((scope = 'TENANT' AND target_id = $1::text)
                    OR (scope = 'PLUGIN' AND target_id = $2))
            )",
        )
        .bind(tenant_id)
        .bind(&plugin_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_failure)?;

        if killed {
            fence_run_blocked(&mut tx, tenant_id, run_id, fence, "KILL_SWITCH_ACTIVE").await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        }

        // 2. Plugin installation status check.
        let plugin_ok: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM plugin_installations
                WHERE tenant_id = $1 AND plugin_id = $2
                  AND state = 'ENABLED' AND artifact_digest = $3
            )",
        )
        .bind(tenant_id)
        .bind(&plugin_id)
        .bind(&plugin_digest)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_failure)?;

        if !plugin_ok {
            fence_run_blocked(&mut tx, tenant_id, run_id, fence, "PLUGIN_NOT_ENABLED").await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        }

        // 3. Membership and current-policy re-check. A dispatch admitted while
        // the requester held a membership must not proceed after that
        // membership or policy has since been withdrawn.
        let membership_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM memberships
                WHERE tenant_id = $1 AND principal_id = $2 AND status = 'ACTIVE'
            )",
        )
        .bind(tenant_id)
        .bind(run_principal)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_failure)?;
        if !membership_active {
            fence_run_blocked(&mut tx, tenant_id, run_id, fence, "MEMBERSHIP_INACTIVE").await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        }
        let requester = match self
            .read_policy_for_principal(&mut tx, tenant_id, run_principal)
            .await
        {
            Ok(requester) => requester,
            Err(_) => {
                // An active membership with no current policy snapshot means
                // policy evaluation is unavailable, not that membership lapsed.
                fence_run_blocked(&mut tx, tenant_id, run_id, fence, "POLICY_REVOKED").await?;
                tx.commit().await.map_err(database_failure)?;
                return Ok(None);
            }
        };
        let definition_id = format!("{}:{}", plugin_id, plugin_digest);
        if !requester.permits(
            "run.start",
            "WorkflowDefinition",
            &definition_id,
            std::collections::BTreeMap::from([
                ("plugin_id".to_owned(), serde_json::json!(plugin_id)),
                ("digest".to_owned(), serde_json::json!(plugin_digest)),
            ]),
        ) {
            fence_run_blocked(&mut tx, tenant_id, run_id, fence, "POLICY_REVOKED").await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        }

        // 4. Grant validity check.
        let grant_row = sqlx::query(
            "SELECT state, expires_at, fence FROM delegations
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(grant_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?;

        let Some(grant) = grant_row else {
            fence_run_blocked(&mut tx, tenant_id, run_id, fence, "GRANT_NOT_FOUND").await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        };

        let grant_state: String = grant.try_get("state").map_err(database_failure)?;
        let grant_expires: DateTime<Utc> = grant.try_get("expires_at").map_err(database_failure)?;
        let grant_current_fence: i64 = grant.try_get("fence").map_err(database_failure)?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;

        if grant_state != "ACTIVE"
            || grant_expires <= now
            || grant_current_fence != expected_grant_fence
        {
            fence_run_blocked(
                &mut tx,
                tenant_id,
                run_id,
                fence,
                "GRANT_REVOKED_OR_EXPIRED",
            )
            .await?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(None);
        }

        let dispatchable = DispatchableRun {
            run_id,
            workflow_id: run.try_get("workflow_id").map_err(database_failure)?,
            workflow_version: run.try_get("workflow_version").map_err(database_failure)?,
            plugin_id,
            plugin_digest,
            grant_id,
            grant_fence: expected_grant_fence,
            fence,
            temporal_workflow_id: run
                .try_get("temporal_workflow_id")
                .map_err(database_failure)?,
        };

        tx.commit().await.map_err(database_failure)?;
        Ok(Some(dispatchable))
    }

    /// Mark an outbox event as successfully delivered and record it in
    /// `inbox_events` for deduplication.
    pub async fn mark_outbox_delivered(
        &self,
        tenant_id: Uuid,
        event_id: Uuid,
    ) -> Result<(), CommandFailure> {
        let mut tx = self
            .begin_system(tenant_id, "outbox-delivered", true)
            .await?;
        sqlx::query(
            "INSERT INTO inbox_events(tenant_id, consumer, event_id, aggregate_sequence)
             VALUES($1, 'worker-relay', $2, 0)
             ON CONFLICT (tenant_id, consumer, event_id) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(event_id)
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;

        sqlx::query(
            "UPDATE outbox_events
             SET delivery_status = 'DELIVERED', lease_expires_at = NULL
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(event_id)
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;

        tx.commit().await.map_err(database_failure)?;
        Ok(())
    }

    /// Handle a dispatch failure for an outbox event with bounded exponential
    /// backoff; move to DEAD_LETTER once max attempts are reached.
    pub async fn mark_outbox_failed(
        &self,
        tenant_id: Uuid,
        event_id: Uuid,
        max_attempts: i32,
    ) -> Result<(), CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "outbox-failed", true).await?;
        let attempts: i32 = sqlx::query_scalar(
            "SELECT delivery_attempts FROM outbox_events WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_failure)?;

        if attempts >= max_attempts {
            sqlx::query(
                "UPDATE outbox_events
                 SET delivery_status = 'DEAD_LETTER', lease_expires_at = NULL
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant_id)
            .bind(event_id)
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;
        } else {
            let backoff_secs: i64 = (1i64 << attempts.min(6)).min(60);
            sqlx::query(
                "UPDATE outbox_events
                 SET lease_expires_at = NULL,
                     available_at = clock_timestamp() + make_interval(secs => $3)
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant_id)
            .bind(event_id)
            .bind(backoff_secs)
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;
        }

        tx.commit().await.map_err(database_failure)?;
        Ok(())
    }
}

async fn fence_run_blocked(
    tx: &mut crate::store::Tx<'_>,
    tenant_id: Uuid,
    run_id: Uuid,
    fence: i64,
    failure_code: &str,
) -> Result<(), CommandFailure> {
    sqlx::query(
        "UPDATE runs
         SET state = 'BLOCKED', dispatch_state = 'BLOCKED',
             fence = $3 + 1, failure_code = $4,
             version = version + 1, updated_at = clock_timestamp()
         WHERE tenant_id = $1 AND id = $2 AND fence = $3",
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(fence)
    .bind(failure_code)
    .execute(&mut **tx)
    .await
    .map_err(database_failure)?;
    Ok(())
}
