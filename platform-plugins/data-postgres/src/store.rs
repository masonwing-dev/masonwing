use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use masonwing_contract_validation::{SchemaCatalog, canonical_bytes, digest_bytes, fingerprint};
use masonwing_contracts::{
    ResourceId,
    wire::{ArtifactRef, Classification, CommandReceipt, ReceiptState, ResourceRef},
};
use masonwing_kernel::runtime::{
    ArtifactObjects, ArtifactScanner, AuthorizedCommand, CommandActor, CommandFailure,
    CommandRepository, FailureKind, PageQuery, PurePluginRuntime, UnavailableArtifactScanner,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

pub(crate) type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone)]
pub struct PostgresStore {
    pub(crate) pool: PgPool,
    pub(crate) objects: Arc<dyn ArtifactObjects>,
    pub(crate) scanner: Arc<dyn ArtifactScanner>,
    pub(crate) spool_directory: PathBuf,
    pub(crate) upload_permits: Arc<tokio::sync::Semaphore>,
    pub(crate) invite_key: Option<crate::InviteTokenKey>,
    pub(crate) known_capabilities: Arc<BTreeSet<String>>,
    pub(crate) native_packages: Arc<BTreeSet<(String, String)>>,
    pub(crate) plugin_runtime: Option<Arc<dyn PurePluginRuntime>>,
}

#[derive(Clone, Debug)]
pub struct ProjectionChange {
    pub resource: ResourceRef,
    pub value: Value,
    pub label: String,
    /// Host-resolved provenance for contracts that carry a plain artifact ID
    /// rather than a complete ArtifactRef, such as a pending UploadSession.
    pub source_artifacts: Vec<Uuid>,
}

/// A committed command or system-path change: the primary projection change
/// plus its durable-run/effect linkage. Public because the system-path budget
/// ledger returns it to worker callers.
#[derive(Debug)]
pub struct Mutation {
    pub primary: ResourceRef,
    pub changes: Vec<ProjectionChange>,
    pub run_id: Option<Uuid>,
    pub effect_id: Option<Uuid>,
    pub accepted: bool,
}

impl Mutation {
    pub(crate) fn one(
        kind: &str,
        id: Uuid,
        version: i32,
        value: Value,
        label: impl Into<String>,
    ) -> Self {
        let resource = resource_ref(kind, id, version);
        Self {
            primary: resource.clone(),
            changes: vec![ProjectionChange {
                resource,
                value,
                label: label.into(),
                source_artifacts: Vec::new(),
            }],
            run_id: None,
            effect_id: None,
            accepted: false,
        }
    }

    pub(crate) fn with_source_artifact(mut self, id: Uuid) -> Self {
        for change in &mut self.changes {
            change.source_artifacts.push(id);
        }
        self
    }
}

impl PostgresStore {
    pub async fn connect(
        database_url: &str,
        objects: Arc<dyn ArtifactObjects>,
    ) -> Result<Self, CommandFailure> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await
            .map_err(database_failure)?;
        Self::from_pool(pool, objects).await
    }

    pub async fn from_pool(
        pool: PgPool,
        objects: Arc<dyn ArtifactObjects>,
    ) -> Result<Self, CommandFailure> {
        let role =
            sqlx::query("SELECT rolsuper,rolbypassrls FROM pg_roles WHERE rolname=current_user")
                .fetch_one(&pool)
                .await
                .map_err(database_failure)?;
        if role
            .try_get::<bool, _>("rolsuper")
            .map_err(database_failure)?
            || role
                .try_get::<bool, _>("rolbypassrls")
                .map_err(database_failure)?
        {
            return Err(CommandFailure::unavailable("RUNTIME_DATABASE_ROLE_UNSAFE"));
        }
        let unsafe_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner WHERE n.nspname='public' AND c.relname=ANY($1) AND (NOT c.relrowsecurity OR NOT c.relforcerowsecurity OR r.rolname=current_user)")
            .bind(vec!["tenants","memberships","command_receipts","artifacts","runs","effects","cost_reservations"])
            .fetch_one(&pool).await.map_err(database_failure)?;
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_tables WHERE schemaname='public' AND tablename=ANY($1)",
        )
        .bind(vec![
            "command_receipts",
            "artifacts",
            "runs",
            "effects",
            "cost_reservations",
        ])
        .fetch_one(&pool)
        .await
        .map_err(database_failure)?;
        if unsafe_count != 0 || tables != 5 {
            return Err(CommandFailure::unavailable("RUNTIME_SCHEMA_UNAVAILABLE"));
        }
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?;
        let known_capabilities = masonwing_contracts::PLATFORM_OPERATIONS
            .iter()
            .copied()
            .chain(["resource.read", "artifact.read", "run.read", "event.read"])
            .map(str::to_owned)
            .collect();
        Ok(Self {
            pool,
            objects,
            scanner: Arc::new(UnavailableArtifactScanner),
            spool_directory: std::env::temp_dir(),
            upload_permits: Arc::new(tokio::sync::Semaphore::new(4)),
            invite_key: None,
            known_capabilities: Arc::new(known_capabilities),
            native_packages: Arc::new(BTreeSet::new()),
            plugin_runtime: None,
        })
    }

    pub fn with_upload_services(
        mut self,
        scanner: Arc<dyn ArtifactScanner>,
        spool_directory: PathBuf,
    ) -> Self {
        self.scanner = scanner;
        self.spool_directory = spool_directory;
        self
    }

    pub fn with_invite_key(mut self, key: crate::InviteTokenKey) -> Self {
        self.invite_key = Some(key);
        self
    }

    /// The composition root supplies reviewed native package digests. No HTTP
    /// command can add entries, and an installed signature alone grants no
    /// in-process native trust.
    pub fn with_native_packages(
        mut self,
        packages: impl IntoIterator<Item = (masonwing_contracts::PluginId, masonwing_contracts::Digest)>,
    ) -> Self {
        self.native_packages = Arc::new(
            packages
                .into_iter()
                .map(|(id, digest)| (id.to_string(), digest.to_string()))
                .collect(),
        );
        self
    }

    /// Extend only from host-registered public contracts, never from a submitted
    /// plugin's own capability requests.
    pub fn with_registered_capabilities(
        mut self,
        actions: impl IntoIterator<Item = masonwing_contracts::Action>,
    ) -> Self {
        Arc::make_mut(&mut self.known_capabilities)
            .extend(actions.into_iter().map(|a| a.to_string()));
        self
    }

    pub fn with_plugin_runtime(mut self, runtime: Arc<dyn PurePluginRuntime>) -> Self {
        self.plugin_runtime = Some(runtime);
        self
    }

    pub(crate) async fn record_change(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        action: &str,
        change: &ProjectionChange,
        correlation_id: Uuid,
    ) -> Result<ArtifactRef, CommandFailure> {
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let principal = parse_uuid(actor.principal_id.as_str())?;
        let artifact = self.write_projection(tx, actor, change).await?;
        sqlx::query("INSERT INTO audit_events(tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(tenant).bind(Uuid::new_v4()).bind(principal).bind(action)
            .bind(&change.resource.resource_type).bind(parse_uuid(change.resource.resource_id.as_str())?).bind(correlation_id)
            .execute(&mut **tx).await.map_err(database_failure)?;
        sqlx::query("INSERT INTO outbox_events(tenant_id,id,aggregate_id,aggregate_version,event_name,artifact_refs,aggregate_type,principal_id,correlation_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(tenant).bind(Uuid::new_v4()).bind(parse_uuid(change.resource.resource_id.as_str())?)
            .bind(change.resource.version).bind(action).bind(json!([artifact]))
            .bind(&change.resource.resource_type).bind(principal).bind(correlation_id)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(artifact)
    }

    /// A shared tenant advisory lock makes the projection watermark refer only
    /// to committed versions. Writers take the exclusive lock through commit;
    /// unrelated tenants proceed independently. RLS remains a separate guard.
    pub(crate) async fn begin(
        &self,
        actor: &CommandActor,
        write: bool,
    ) -> Result<Tx<'static>, CommandFailure> {
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let principal = parse_uuid(actor.principal_id.as_str())?;
        let mut tx = self.pool.begin().await.map_err(database_failure)?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.principal_id',$2,true),set_config('statement_timeout','10000',true),set_config('lock_timeout','5000',true)")
            .bind(tenant.to_string()).bind(principal.to_string()).execute(&mut *tx).await.map_err(database_failure)?;
        let lock = if write {
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))"
        } else {
            "SELECT pg_advisory_xact_lock_shared(hashtextextended($1,0))"
        };
        sqlx::query(lock)
            .bind(format!("masonwing:tenant:{tenant}"))
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;
        let active: Option<bool> =
            sqlx::query_scalar("SELECT status='ACTIVE' FROM tenants WHERE id=$1")
                .bind(tenant)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_failure)?;
        if active != Some(true) {
            return Err(CommandFailure::not_found());
        }
        let membership = sqlx::query("SELECT status,membership_epoch,permission_epoch FROM memberships WHERE tenant_id=$1 AND principal_id=$2 FOR SHARE")
            .bind(tenant).bind(principal).fetch_optional(&mut *tx).await.map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        if membership
            .try_get::<String, _>("status")
            .map_err(database_failure)?
            != "ACTIVE"
            || membership
                .try_get::<i64, _>("membership_epoch")
                .map_err(database_failure)?
                != actor.membership_epoch
            || membership
                .try_get::<i64, _>("permission_epoch")
                .map_err(database_failure)?
                != actor.permission_epoch
        {
            return Err(CommandFailure::denied("AUTHORITY_CHANGED"));
        }
        let policy = sqlx::query("SELECT policy_version,policy_epoch FROM authorization_policies WHERE tenant_id=$1 AND is_current FOR SHARE")
            .bind(tenant).fetch_optional(&mut *tx).await.map_err(database_failure)?
            .ok_or_else(|| CommandFailure::unavailable("POLICY_UNAVAILABLE"))?;
        if policy
            .try_get::<String, _>("policy_version")
            .map_err(database_failure)?
            != actor.policy_version
            || policy
                .try_get::<i64, _>("policy_epoch")
                .map_err(database_failure)?
                != actor.policy_epoch
        {
            return Err(CommandFailure::denied("AUTHORITY_CHANGED"));
        }
        Ok(tx)
    }

    /// System-actor transaction for internal executors (worker dispatch,
    /// effect reconciliation, budget settlement). RLS scope is still set to
    /// the run's tenant, but the principal is a platform role
    /// (`system:{purpose}`) rather than a membership-backed user. The caller
    /// is responsible for re-checking current authority boundaries — this path
    /// does not validate the system actor's membership, because the platform
    /// acts, not the user.
    pub(crate) async fn begin_system(
        &self,
        tenant_id: Uuid,
        purpose: &str,
        write: bool,
    ) -> Result<Tx<'static>, CommandFailure> {
        let mut tx = self.pool.begin().await.map_err(database_failure)?;
        // PostgreSQL RLS policies cast app.principal_id to uuid. Use a
        // deterministic system UUID scoped by purpose.
        let principal = Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("system:{purpose}").as_bytes());
        sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.principal_id',$2,true),set_config('statement_timeout','10000',true),set_config('lock_timeout','5000',true)")
            .bind(tenant_id.to_string()).bind(principal.to_string()).execute(&mut *tx).await.map_err(database_failure)?;
        let lock = if write {
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))"
        } else {
            "SELECT pg_advisory_xact_lock_shared(hashtextextended($1,0))"
        };
        sqlx::query(lock)
            .bind(format!("masonwing:tenant:{tenant_id}"))
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;
        let active: Option<bool> =
            sqlx::query_scalar("SELECT status='ACTIVE' FROM tenants WHERE id=$1")
                .bind(tenant_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_failure)?;
        if active != Some(true) {
            return Err(CommandFailure::not_found());
        }
        Ok(tx)
    }

    pub(crate) async fn write_projection(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        change: &ProjectionChange,
    ) -> Result<ArtifactRef, CommandFailure> {
        if crate::queries::BASELINE_RESOURCE_TYPES.contains(&change.resource.resource_type.as_str())
        {
            SchemaCatalog::shared()
                .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
                .validate(&change.resource.resource_type, &change.value)
                .map_err(|_| CommandFailure::unavailable("PROJECTION_CONTRACT_INVALID"))?;
        }
        let bytes = canonical_bytes(&change.value)
            .map_err(|_| CommandFailure::invalid("PROJECTION_INVALID"))?;
        let mut sources = Vec::new();
        projection_sources(&change.value, &mut sources);
        let mut classification = sources
            .iter()
            .map(|source| source.classification)
            .fold(Classification::Internal, std::cmp::max);
        let mut source_ids: BTreeSet<_> = change.source_artifacts.iter().copied().collect();
        for source in &sources {
            if source.tenant_id != actor.tenant_id {
                return Err(CommandFailure::not_found());
            }
            source_ids.insert(parse_uuid(source.artifact_id.as_str())?);
        }
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        // Consult authoritative metadata even while a source is still PENDING.
        // The submitted projection cannot lower the source's classification.
        for id in &source_ids {
            let source_class: String = sqlx::query_scalar(
                "SELECT classification FROM artifacts WHERE tenant_id=$1 AND id=$2",
            )
            .bind(tenant)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
            let source_class: Classification = serde_json::from_value(json!(source_class))
                .map_err(|_| CommandFailure::unavailable("ARTIFACT_CLASSIFICATION_INVALID"))?;
            classification = classification.max(source_class);
        }
        let reference = self
            .persist_artifact(tx, actor, bytes, "application/json", classification)
            .await?;
        // A metadata projection can expose confidential artifact identifiers and
        // digests. It inherits their classification and deletion lineage too.
        for source in &source_ids {
            sqlx::query("INSERT INTO artifact_lineage(tenant_id,source_id,derived_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(tenant).bind(source)
                .bind(parse_uuid(reference.artifact_id.as_str())?).execute(&mut **tx).await.map_err(database_failure)?;
        }
        sqlx::query("INSERT INTO resource_projections (tenant_id,resource_type,resource_id,version,artifact_id,search_label,created_at) VALUES ($1,$2,$3,$4,$5,$6,COALESCE((SELECT min(created_at) FROM resource_projections WHERE tenant_id=$1 AND resource_type=$2 AND resource_id=$3),clock_timestamp()))")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(&change.resource.resource_type)
            .bind(change.resource.resource_id.as_str()).bind(change.resource.version)
            .bind(parse_uuid(reference.artifact_id.as_str())?).bind(change.label.chars().take(512).collect::<String>())
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(reference)
    }

    pub(crate) async fn persist_artifact(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        bytes: Vec<u8>,
        content_type: &str,
        classification: Classification,
    ) -> Result<ArtifactRef, CommandFailure> {
        let id = Uuid::new_v4();
        let digest = digest_bytes(&bytes);
        let key = format!(
            "{}/{}/{}",
            actor.tenant_id,
            id,
            digest.as_str().trim_start_matches("sha256:")
        );
        let size =
            i64::try_from(bytes.len()).map_err(|_| CommandFailure::invalid("PAYLOAD_TOO_LARGE"))?;
        if !(1..=1_073_741_824).contains(&size) {
            return Err(CommandFailure::invalid("PAYLOAD_TOO_LARGE"));
        }
        // An object uploaded before a rolled-back transaction is unreachable:
        // access always requires ACTIVE tenant-scoped metadata. GC can remove it.
        self.objects
            .put_immutable(&key, bytes, content_type)
            .await?;
        sqlx::query("INSERT INTO artifacts(tenant_id,id,digest,classification,content_type,size_bytes,storage_key,state,creator_id) VALUES($1,$2,$3,$4,$5,$6,$7,'ACTIVE',$8)")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(id).bind(digest.as_str())
            .bind(classification.as_str()).bind(content_type).bind(size).bind(&key)
            .bind(parse_uuid(actor.principal_id.as_str())?).execute(&mut **tx).await.map_err(database_failure)?;
        Ok(ArtifactRef {
            artifact_id: masonwing_contracts::ArtifactId::new(id.to_string()).expect("uuid"),
            tenant_id: actor.tenant_id.clone(),
            digest,
            schema_version: "1.0.0".into(),
            classification,
        })
    }

    pub(crate) async fn load_artifact_ref(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        reference: &ArtifactRef,
        maximum: usize,
    ) -> Result<Vec<u8>, CommandFailure> {
        if reference.tenant_id != actor.tenant_id {
            return Err(CommandFailure::not_found());
        }
        let row = sqlx::query("SELECT digest,classification,schema_version,storage_key,size_bytes FROM artifacts a WHERE tenant_id=$1 AND id=$2 AND state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM deletion_tombstones d WHERE d.tenant_id=a.tenant_id AND d.resource_type='Artifact' AND d.resource_id=a.id::text)")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(parse_uuid(reference.artifact_id.as_str())?)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        if row
            .try_get::<String, _>("digest")
            .map_err(database_failure)?
            != reference.digest.as_str()
            || row
                .try_get::<String, _>("classification")
                .map_err(database_failure)?
                != reference.classification.as_str()
            || row
                .try_get::<String, _>("schema_version")
                .map_err(database_failure)?
                != reference.schema_version
        {
            return Err(CommandFailure::precondition("ARTIFACT_REFERENCE_MISMATCH"));
        }
        let size: i64 = row.try_get("size_bytes").map_err(database_failure)?;
        if size > maximum as i64 {
            return Err(CommandFailure::invalid("PAYLOAD_TOO_LARGE"));
        }
        let bytes = self
            .objects
            .get_bounded(
                &row.try_get::<String, _>("storage_key")
                    .map_err(database_failure)?,
                maximum,
            )
            .await?;
        if bytes.len() as i64 != size || digest_bytes(&bytes) != reference.digest {
            return Err(CommandFailure::artifact_integrity(reference.clone()));
        }
        Ok(bytes)
    }
}

#[async_trait::async_trait]
impl CommandRepository for PostgresStore {
    async fn execute(&self, command: AuthorizedCommand) -> Result<CommandReceipt, CommandFailure> {
        let schema = format!("Command_{}", command.operation.replace(['.', '-'], "_"));
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
            .validate(&schema, &command.input)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        if fingerprint(&command.operation, &command.input)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?
            != command.fingerprint
        {
            return Err(CommandFailure::invalid("FINGERPRINT_MISMATCH"));
        }
        let mut tx = self.begin(&command.actor, true).await?;
        self.authorize_command(&mut tx, &command).await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let previous = sqlx::query("SELECT fingerprint,receipt FROM command_receipts WHERE tenant_id=$1 AND principal_id=$2 AND operation=$3 AND idempotency_key=$4 AND expires_at>clock_timestamp() FOR UPDATE")
            .bind(tenant).bind(principal).bind(&command.operation).bind(command.idempotency_key.as_str())
            .fetch_optional(&mut *tx).await.map_err(database_failure)?;
        if let Some(previous) = previous {
            if previous
                .try_get::<String, _>("fingerprint")
                .map_err(database_failure)?
                != command.fingerprint.as_str()
            {
                return Err(CommandFailure::conflict("IDEMPOTENCY_CONFLICT"));
            }
            let receipt: Value = previous.try_get("receipt").map_err(database_failure)?;
            SchemaCatalog::shared()
                .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
                .validate("CommandReceipt", &receipt)
                .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
            return serde_json::from_value(receipt)
                .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"));
        }
        let mutation = match self.dispatch(&mut tx, &command).await {
            Ok(mutation) => mutation,
            Err(error) => {
                tx.rollback().await.map_err(database_failure)?;
                if let Some(reference) = error.integrity_reference() {
                    // Installation is rolled back first. A separate, reauthorized
                    // containment transaction persists only proven corruption.
                    self.quarantine_corrupt_artifact(&command, reference)
                        .await?;
                }
                return Err(error);
            }
        };
        if mutation.accepted && mutation.run_id.is_none() {
            return Err(CommandFailure::unavailable("DURABLE_RUN_REQUIRED"));
        }
        let command_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let accepted_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
        for change in &mutation.changes {
            let artifact = self
                .write_projection(&mut tx, &command.actor, change)
                .await?;
            let audit_id = Uuid::new_v4();
            sqlx::query("INSERT INTO audit_events(tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(tenant).bind(audit_id).bind(principal).bind(&command.operation)
                .bind(&change.resource.resource_type).bind(parse_uuid(change.resource.resource_id.as_str())?).bind(correlation_id)
                .execute(&mut *tx).await.map_err(database_failure)?;
            sqlx::query("INSERT INTO outbox_events(tenant_id,id,aggregate_id,aggregate_version,event_name,artifact_refs,aggregate_type,principal_id,correlation_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(tenant).bind(Uuid::new_v4()).bind(parse_uuid(change.resource.resource_id.as_str())?)
                .bind(change.resource.version).bind(&command.operation).bind(json!([artifact]))
                .bind(&change.resource.resource_type).bind(principal).bind(correlation_id)
                .execute(&mut *tx).await.map_err(database_failure)?;
        }
        let receipt = CommandReceipt {
            command_id: resource_id(command_id),
            state: if mutation.accepted {
                ReceiptState::Accepted
            } else {
                ReceiptState::Succeeded
            },
            resource: Some(mutation.primary),
            run_id: mutation.run_id.map(resource_id),
            effect_id: mutation.effect_id.map(resource_id),
            correlation_id: resource_id(correlation_id),
            accepted_at,
        };
        let value = serde_json::to_value(&receipt)
            .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
            .validate("CommandReceipt", &value)
            .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
        sqlx::query("INSERT INTO command_receipts(tenant_id,principal_id,operation,idempotency_key,fingerprint,command_id,receipt,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,clock_timestamp()+interval '24 hours') ON CONFLICT(tenant_id,principal_id,operation,idempotency_key) DO UPDATE SET fingerprint=EXCLUDED.fingerprint,command_id=EXCLUDED.command_id,receipt=EXCLUDED.receipt,expires_at=EXCLUDED.expires_at,created_at=clock_timestamp() WHERE command_receipts.expires_at<=clock_timestamp()")
            .bind(tenant).bind(principal).bind(&command.operation).bind(command.idempotency_key.as_str())
            .bind(command.fingerprint.as_str()).bind(command_id).bind(value)
            .execute(&mut *tx).await.map_err(database_failure)?;
        tx.commit().await.map_err(database_failure)?;
        Ok(receipt)
    }

    async fn collection(
        &self,
        actor: &CommandActor,
        resource_type: &str,
        page: &PageQuery,
    ) -> Result<masonwing_contracts::wire::ReadCollection, CommandFailure> {
        self.read_collection(actor, resource_type, page).await
    }

    async fn projection(
        &self,
        actor: &CommandActor,
        resource_type: &str,
        id: &str,
    ) -> Result<masonwing_contracts::wire::ResourceProjection, CommandFailure> {
        self.read_projection(actor, resource_type, id).await
    }
}

pub(crate) fn parse_uuid(value: &str) -> Result<Uuid, CommandFailure> {
    Uuid::parse_str(value).map_err(|_| CommandFailure::not_found())
}
pub(crate) fn resource_id(id: Uuid) -> ResourceId {
    ResourceId::new(id.to_string()).expect("uuid")
}
pub(crate) fn resource_ref(kind: &str, id: Uuid, version: i32) -> ResourceRef {
    ResourceRef {
        resource_type: kind.into(),
        resource_id: resource_id(id),
        version,
    }
}
pub(crate) fn database_failure(error: sqlx::Error) -> CommandFailure {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("23505") => CommandFailure::conflict("RESOURCE_CONFLICT"),
        Some("23503") => CommandFailure::not_found(),
        Some("23514" | "22003" | "22001") => CommandFailure::invalid("CONSTRAINT_INVALID"),
        Some("40001" | "40P01") => CommandFailure::conflict("CONCURRENT_UPDATE"),
        Some("55P03") => CommandFailure::new("RESOURCE_BUSY", FailureKind::Exhausted),
        _ => CommandFailure::unavailable("DATA_ADAPTER_UNAVAILABLE"),
    }
}

pub(crate) fn field<T: serde::de::DeserializeOwned>(
    input: &Value,
    name: &str,
) -> Result<T, CommandFailure> {
    serde_json::from_value(
        input
            .get(name)
            .cloned()
            .ok_or_else(|| CommandFailure::invalid("SCHEMA_INVALID"))?,
    )
    .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))
}

fn projection_sources(value: &Value, sources: &mut Vec<ArtifactRef>) {
    match value {
        Value::Object(object) => {
            if object.contains_key("artifact_id")
                && object.contains_key("digest")
                && let Ok(reference) = serde_json::from_value::<ArtifactRef>(value.clone())
            {
                sources.push(reference);
                return;
            }
            for child in object.values() {
                projection_sources(child, sources);
            }
        }
        Value::Array(array) => {
            for child in array {
                projection_sources(child, sources);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use masonwing_contracts::{ArtifactId, TenantId};

    #[test]
    fn nested_artifact_refs_keep_confidential_classification_without_a_magic_field_count() {
        let source = ArtifactRef {
            artifact_id: ArtifactId::new(Uuid::new_v4().to_string()).unwrap(),
            tenant_id: TenantId::new(Uuid::new_v4().to_string()).unwrap(),
            digest: digest_bytes(b"secret"),
            schema_version: "1.0.0".into(),
            classification: Classification::Confidential,
        };
        let mut found = Vec::new();
        projection_sources(
            &json!({"output_ref":source,"nested":[{"input_ref":source}]}),
            &mut found,
        );
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|reference| reference == &source));
        assert_eq!(
            found
                .iter()
                .map(|reference| reference.classification)
                .fold(Classification::Internal, std::cmp::max),
            Classification::Confidential
        );
    }
}
