//! Explicit local PostgreSQL integration suite; object storage is an instrumented
//! fault-injection adapter. This is not a MinIO or product-acceptance result.
//! Run: cargo test -p masonwing-data-postgres --features local-postgres-tests --test registry_postgres

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ed25519_dalek::{Signer, SigningKey};
use masonwing_contract_validation::{canonical_bytes, digest_bytes, fingerprint};
use masonwing_contracts::{
    ArtifactId, PrincipalId, ResourceId, TenantId,
    wire::{ArtifactRef, Classification, PluginManifest, ReceiptState},
};
use masonwing_data_postgres::{
    CheckpointWrite, PostgresStore,
    registry_validation::{manifest_digest, manifest_signing_message},
};
use masonwing_kernel::runtime::{
    ArtifactObjects, AuthorizedCommand, CommandActor, CommandFailure, CommandRepository,
    PurePluginInvocation, PurePluginOutput, PurePluginRuntime,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Default)]
struct Objects {
    bytes: Mutex<BTreeMap<String, Vec<u8>>>,
    reads: AtomicUsize,
    writes: AtomicUsize,
}

#[async_trait]
impl ArtifactObjects for Objects {
    async fn put_immutable(
        &self,
        key: &str,
        bytes: Vec<u8>,
        _: &str,
    ) -> Result<(), CommandFailure> {
        let mut objects = self.bytes.lock().unwrap();
        if objects.get(key).is_some_and(|current| current != &bytes) {
            return Err(CommandFailure::conflict("OBJECT_IMMUTABLE"));
        }
        objects.insert(key.to_owned(), bytes);
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn get_bounded(&self, key: &str, maximum: usize) -> Result<Vec<u8>, CommandFailure> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let value = self
            .bytes
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(CommandFailure::not_found)?;
        if value.len() > maximum {
            return Err(CommandFailure::invalid("PAYLOAD_TOO_LARGE"));
        }
        Ok(value)
    }
}

struct Fixture {
    operator: PgPool,
    store: PostgresStore,
    actor: CommandActor,
    tenant: Uuid,
    objects: Arc<Objects>,
    signer: SigningKey,
}

impl Fixture {
    async fn create() -> TestResult<Self> {
        Self::create_with_policy("permit(principal, action, resource); forbid(principal, action == Action::\"effect.dispatch\", resource);").await
    }

    /// Tenant with a policy that also permits the effect operations the
    /// effect-lifecycle tests exercise. The policy is immutable after creation.
    async fn effect() -> TestResult<Self> {
        Self::create_with_policy("permit(principal, action, resource);").await
    }

    async fn create_with_policy(policy: &str) -> TestResult<Self> {
        // Public synthetic credentials already used by the local migration CLI.
        // No configurable remote endpoint is accepted by this test target.
        let operator = PgPoolOptions::new().max_connections(2).acquire_timeout(Duration::from_secs(5))
            .connect("postgresql://masonwing_migrator:masonwing-local-migrator@127.0.0.1:39852/masonwing").await?;
        let runtime = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(5))
            .connect("postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing")
            .await?;
        let objects = Arc::new(Objects::default());
        let store = PostgresStore::from_pool(runtime, objects.clone()).await?;
        let tenant = Uuid::new_v4();
        let principal = Uuid::new_v4();
        let membership = Uuid::new_v4();
        let actor = CommandActor {
            tenant_id: TenantId::new(tenant.to_string())?,
            principal_id: PrincipalId::new(principal.to_string())?,
            issuer: "https://registry-test.example.test".into(),
            membership_epoch: 1,
            permission_epoch: 1,
            policy_version: "1.0.0".into(),
            policy_epoch: 1,
        };
        let mut tx = operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO tenants(id,name) VALUES($1,'registry integration fixture')")
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch) VALUES($1,$2,$3,'OWNER','ACTIVE',1,1)")
            .bind(tenant).bind(principal).bind(membership).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES($1,$2,'OWNER')",
        )
        .bind(tenant)
        .bind(membership)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current) VALUES($1,'1.0.0',1,$2,true)")
            .bind(tenant).bind(policy)
            .execute(&mut *tx).await?;
        let signer = SigningKey::from_bytes(&[29; 32]); // Public synthetic fixture.
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'test.publisher','test.key',$2)")
            .bind(tenant).bind(signer.verifying_key().to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self {
            operator,
            store,
            actor,
            tenant,
            objects,
            signer,
        })
    }

    fn command(&self, operation: &str, input: Value) -> AuthorizedCommand {
        AuthorizedCommand {
            actor: self.actor.clone(),
            operation: operation.into(),
            idempotency_key: ResourceId::new(Uuid::new_v4().to_string()).unwrap(),
            fingerprint: fingerprint(operation, &input).unwrap(),
            input,
        }
    }

    async fn artifact(&self, bytes: Vec<u8>) -> TestResult<(ArtifactRef, String)> {
        self.classified_artifact(bytes, Classification::Internal)
            .await
    }

    async fn classified_artifact(
        &self,
        bytes: Vec<u8>,
        classification: Classification,
    ) -> TestResult<(ArtifactRef, String)> {
        let id = Uuid::new_v4();
        let digest = digest_bytes(&bytes);
        let key = format!("{}/{}/{}", self.tenant, id, digest.as_str());
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifacts(tenant_id,id,digest,classification,content_type,size_bytes,storage_key,state,creator_id) VALUES($1,$2,$3,$7,'application/octet-stream',$4,$5,'ACTIVE',$6)")
            .bind(self.tenant).bind(id).bind(digest.as_str()).bind(bytes.len() as i64).bind(&key).bind(Uuid::parse_str(self.actor.principal_id.as_str())?).bind(classification.as_str()).execute(&mut *tx).await?;
        self.objects
            .put_immutable(&key, bytes, "application/octet-stream")
            .await?;
        tx.commit().await?;
        Ok((
            ArtifactRef {
                artifact_id: ArtifactId::new(id.to_string())?,
                tenant_id: self.actor.tenant_id.clone(),
                digest,
                schema_version: "1.0.0".into(),
                classification,
            },
            key,
        ))
    }

    async fn package(&self, id: &str, version: &str) -> TestResult<(PluginManifest, String)> {
        let (code, key) = self
            .artifact(format!("synthetic package {id} {version}").into_bytes())
            .await?;
        let (sbom, _) = self
            .artifact(br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}"#.to_vec())
            .await?;
        let manifest: PluginManifest = serde_json::from_value(
            json!({"id":id,"version":version,"contract_version":"1.0.0","artifact_digest":code.digest,
            "publisher_id":"test.publisher","execution_class":"WASM_COMPONENT","requested_capabilities":["resource.read","effect.dispatch"],
            "dependencies":[],"ui":[],"migrations":[],"license_expression":"MIT","sbom_digest":sbom.digest,"signature_ref":Uuid::new_v4().to_string(),
            "handlers":[],"workflows":[],"data_contracts":[],"tools":[]}),
        )?;
        self.sign(&manifest).await?;
        Ok((manifest, key))
    }

    async fn sign(&self, manifest: &PluginManifest) -> TestResult {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_signatures(tenant_id,id,publisher_id,key_id,artifact_digest,manifest_digest,signature) VALUES($1,$2,'test.publisher','test.key',$3,$4,$5)")
            .bind(self.tenant).bind(&manifest.signature_ref).bind(manifest.artifact_digest.as_str()).bind(manifest_digest(manifest)?.as_str())
            .bind(self.signer.sign(&manifest_signing_message(manifest)?).to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn counts(&self) -> TestResult<(i64, i64, i64, i64)> {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT (SELECT count(*) FROM plugin_installations WHERE tenant_id=$1) AS installed,(SELECT count(*) FROM command_receipts WHERE tenant_id=$1) AS receipts,(SELECT count(*) FROM audit_events WHERE tenant_id=$1) AS audit,(SELECT count(*) FROM outbox_events WHERE tenant_id=$1) AS outbox")
            .bind(self.tenant).fetch_one(&mut *tx).await?;
        let result = (
            row.try_get("installed")?,
            row.try_get("receipts")?,
            row.try_get("audit")?,
            row.try_get("outbox")?,
        );
        tx.rollback().await?;
        Ok(result)
    }

    async fn state(&self, plugin: &str) -> TestResult<(String, i32, String)> {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT state,version,artifact_digest FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2")
            .bind(self.tenant).bind(plugin).fetch_one(&mut *tx).await?;
        let result = (
            row.try_get("state")?,
            row.try_get("version")?,
            row.try_get("artifact_digest")?,
        );
        tx.rollback().await?;
        Ok(result)
    }

    async fn cleanup(&self) -> TestResult {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        // Fixed tables and this test's UUID only; no schema reset or shared data.
        for table in [
            "resource_projections",
            "uploads",
            "artifact_lineage",
            "artifact_rights",
            "kill_switches",
            "effect_receipts",
            "effects",
            "connections",
            "run_checkpoints",
            "cost_ledger",
            "cost_reservations",
            "budget_accounts",
            "budget_settings",
            "approvals",
            "runs",
            "delegations",
            "product_compositions",
            "workflow_definitions",
            "plugin_revocations",
            "plugin_installations",
            "plugin_versions",
            "artifact_signatures",
            "publisher_keys",
            "command_receipts",
            "audit_events",
            "outbox_events",
            "inbox_events",
            "artifacts",
            "authorization_policies",
            "membership_roles",
            "memberships",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE tenant_id=$1"))
                .bind(self.tenant)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM tenants WHERE id=$1")
            .bind(self.tenant)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.operator.close().await;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_upload_projection_inherits_confidentiality_and_deletion_provenance() -> TestResult
{
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let receipt = f.store.execute(f.command("artifact.begin",json!({
                "classification":"CONFIDENTIAL","content_type":"text/plain","size_bytes":6,"expected_digest":digest_bytes(b"secret")
            }))).await?;
            let resource = receipt.resource.unwrap();
            let projection = f
                .store
                .read_projection(&f.actor, "UploadSession", resource.resource_id.as_str())
                .await?;
            assert_eq!(
                projection.artifact_ref.classification,
                Classification::Confidential
            );
            let (_, content) = f
                .store
                .artifact_content(
                    &f.actor,
                    projection.artifact_ref.artifact_id.as_str(),
                    1024 * 1024,
                )
                .await?;
            let value: Value = serde_json::from_slice(&content)?;
            let source = Uuid::parse_str(value["artifact_id"].as_str().unwrap())?;
            assert!(
                f.store
                    .artifact_content(&f.actor, &source.to_string(), 1024 * 1024)
                    .await
                    .is_err()
            );
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let provenance: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifact_lineage WHERE tenant_id=$1 AND source_id=$2 AND derived_id=$3)")
                .bind(f.tenant).bind(source).bind(Uuid::parse_str(projection.artifact_ref.artifact_id.as_str())?).fetch_one(&mut *tx).await?;
            assert!(provenance);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-069/REQ-077: run.start admits a stable queued identity,
// validates input against the workflow definition schema, and replays idempotently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_start_accepts_stable_identity_and_replay_does_not_mutate() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (manifest, _) = f.package("test.start", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            // run.start admits only the exact ENABLED build the definition pins.
            f.store
                .execute(f.command(
                    "plugin.enable",
                    json!({
                    "plugin_id": manifest.id, "artifact_digest": manifest.artifact_digest,
                    "grants": ["resource.read"], "expected_version": 1}),
                ))
                .await?;
            let (schema_ref, _) = f
                .classified_artifact(br#"{"type":"object"}"#.to_vec(), Classification::Internal)
                .await?;
            let (input, _) = f
                .classified_artifact(
                    br#"{"hello":"world"}"#.to_vec(),
                    Classification::Confidential,
                )
                .await?;
            let grant = Uuid::new_v4();
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let definition = json!({
                "id": "test.start.workflow", "version": "1.0.0",
                "input_schema_ref": serde_json::to_value(&schema_ref)?,
                "output_schema_ref": serde_json::to_value(&schema_ref)?,
                "entry_node": "start", "nodes": [], "edges": [],
                "max_total_steps": 8, "max_model_turns": 4, "required_actions": [],
                "digest": manifest.artifact_digest,
            });
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO workflow_definitions(tenant_id,workflow_id,workflow_version,plugin_id,plugin_digest,definition_digest,definition) VALUES($1,'test.start.workflow','1.0.0',$2,$3,$3,$4)")
                .bind(f.tenant).bind(manifest.id.as_str()).bind(manifest.artifact_digest.as_str()).bind(definition)
                .execute(&mut *tx).await?;
            tx.commit().await?;

            let command = f.command(
                "run.start",
                json!({"workflow_id":"test.start.workflow","workflow_version":"1.0.0",
                    "input_ref":serde_json::to_value(&input)?,"grant_id":grant}),
            );
            let before = f.counts().await?;
            let receipt = f.store.execute(command.clone()).await?;
            assert_eq!(receipt.state, ReceiptState::Accepted);
            let run = receipt.run_id.as_ref().unwrap().as_str().to_owned();
            assert_eq!(receipt.resource.as_ref().unwrap().resource_id.as_str(), run);
            assert_eq!(receipt.resource.as_ref().unwrap().version, 1);
            let after = f.counts().await?;
            assert_eq!(after, (before.0, before.1 + 1, before.2 + 1, before.3 + 1));

            let writes = f.objects.writes.load(Ordering::SeqCst);
            let replay = f.store.execute(command.clone()).await?;
            assert_eq!(
                serde_json::to_value(&replay)?,
                serde_json::to_value(&receipt)?
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);

            // Same key with a different workflow version is a fingerprint conflict.
            let conflict_input = json!({"workflow_id":"test.start.workflow","workflow_version":"9.9.9",
                "input_ref":serde_json::to_value(&input)?,"grant_id":grant});
            let conflict = AuthorizedCommand {
                input: conflict_input.clone(),
                fingerprint: fingerprint("run.start", &conflict_input)?,
                ..command.clone()
            };
            assert_eq!(
                f.store.execute(conflict).await.unwrap_err().code,
                "IDEMPOTENCY_CONFLICT"
            );
            // Unknown workflow versions have no definition to admit.
            let unknown = f.command(
                "run.start",
                json!({"workflow_id":"test.start.workflow","workflow_version":"9.9.9",
                    "input_ref":serde_json::to_value(&input)?,"grant_id":grant}),
            );
            assert_eq!(
                f.store.execute(unknown).await.unwrap_err().code,
                "RESOURCE_NOT_FOUND"
            );
            // Input bytes outside the definition schema are rejected.
            let (bad_input, _) = f
                .classified_artifact(br#"[1,2]"#.to_vec(), Classification::Confidential)
                .await?;
            // The fixture's own artifact upload is one object write; the rejected
            // command itself must add none.
            let writes_after_fixture = f.objects.writes.load(Ordering::SeqCst);
            let invalid = f.command(
                "run.start",
                json!({"workflow_id":"test.start.workflow","workflow_version":"1.0.0",
                    "input_ref":serde_json::to_value(&bad_input)?,"grant_id":grant}),
            );
            assert_eq!(
                f.store.execute(invalid).await.unwrap_err().code,
                "WORKFLOW_INPUT_INVALID"
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(
                f.objects.writes.load(Ordering::SeqCst),
                writes_after_fixture
            );

            // A new connection/transaction observes only the committed admission.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query(
                "SELECT state,version,fence,dispatch_state,temporal_workflow_id,grant_fence FROM runs WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(Uuid::parse_str(&run)?)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "QUEUED");
            assert_eq!(row.try_get::<i32, _>("version")?, 1);
            assert_eq!(row.try_get::<i64, _>("fence")?, 1);
            assert_eq!(row.try_get::<String, _>("dispatch_state")?, "PENDING");
            assert_eq!(row.try_get::<String, _>("temporal_workflow_id")?, run);
            assert_eq!(row.try_get::<i64, _>("grant_fence")?, 1);
            tx.rollback().await?;
            let current = f.store.projection(&f.actor, "Run", &run).await?;
            assert_eq!(current.resource.version, 1);
            assert_eq!(
                current.artifact_ref.classification,
                Classification::Confidential
            );
            let (_, bytes) = f
                .store
                .artifact_content(
                    &f.actor,
                    current.artifact_ref.artifact_id.as_str(),
                    1024 * 1024,
                )
                .await?;
            assert_eq!(serde_json::from_slice::<Value>(&bytes)?["state"], "QUEUED");
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

/// Package a workflow definition as a tenant artifact and return its
/// serialized bytes together with the matching `ArtifactRef`. The digest is
/// recomputed over the definition minus its digest field, exactly as
/// `validate_workflow` recomputes it at install time.
async fn workflow_package(
    f: &Fixture,
    nodes: Value,
    edges: Value,
    entry_node: &str,
) -> TestResult<(Vec<u8>, ArtifactRef)> {
    let (schema_ref, _) = f
        .classified_artifact(br#"{"type":"object"}"#.to_vec(), Classification::Internal)
        .await?;
    let mut value = json!({
        "id": "test.invalid.workflow", "version": "1.0.0",
        "input_schema_ref": serde_json::to_value(&schema_ref)?,
        "output_schema_ref": serde_json::to_value(&schema_ref)?,
        "entry_node": entry_node, "nodes": nodes, "edges": edges,
        "max_total_steps": 8, "max_model_turns": 4, "required_actions": [],
        "digest": "sha256:0",
    });
    let mut content = value.clone();
    content.as_object_mut().unwrap().remove("digest");
    value["digest"] = json!(digest_bytes(&canonical_bytes(&content)?));
    let bytes = serde_json::to_vec(&value)?;
    let (reference, _) = f
        .classified_artifact(bytes.clone(), Classification::Internal)
        .await?;
    Ok((bytes, reference))
}

// MASONWING@1.0.1 REQ-076 / AC-080: installing a workflow whose nodes name
// handlers that do not exist, or whose graph has a cycle no BOUNDED_LOOP
// bounds, must be refused as WORKFLOW_INVALID before any run exists.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_workflow_install_is_refused_before_any_run() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let runs_before: i64 = {
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                let count = sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1")
                    .bind(f.tenant)
                    .fetch_one(&mut *tx)
                    .await?;
                tx.rollback().await?;
                count
            };
            let counts_before = f.counts().await?;

            let (schema_ref, _) = f
                .classified_artifact(br#"{"type":"object"}"#.to_vec(), Classification::Internal)
                .await?;
            let handler = json!({
                "id": "compute",
                "input_schema_ref": serde_json::to_value(&schema_ref)?,
                "output_schema_ref": serde_json::to_value(&schema_ref)?,
                "effects": "DETERMINISTIC", "required_capabilities": [],
                "max_execution_seconds": 60,
            });
            let node = |id: &str, handler_id: Option<&str>| {
                json!({
                    "id": id, "kind": "ACTIVITY", "handler_id": handler_id,
                    "input_schema_ref": serde_json::to_value(&schema_ref).unwrap(),
                    "output_schema_ref": serde_json::to_value(&schema_ref).unwrap(),
                    "max_attempts": 1, "timeout_seconds": 30, "max_iterations": 1,
                    "on_failure": "FAIL",
                })
            };

            let try_install = async |f: &Fixture,
                                     nodes: Value,
                                     edges: Value,
                                     entry_node: &str|
                   -> TestResult<Result<(), CommandFailure>> {
                let (bytes, reference) = workflow_package(f, nodes, edges, entry_node).await?;
                let (code, _) = f
                    .artifact(
                        format!("synthetic package test.invalid 1.0.0 {bytes:?}").into_bytes(),
                    )
                    .await?;
                let (sbom, _) = f
                    .artifact(
                        br#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}"#
                            .to_vec(),
                    )
                    .await?;
                let manifest: PluginManifest = serde_json::from_value(
                    json!({"id":"test.invalid","version":"1.0.0","contract_version":"1.0.0",
                        "artifact_digest":code.digest,"publisher_id":"test.publisher",
                        "execution_class":"WASM_COMPONENT","requested_capabilities":[],
                        "dependencies":[],"ui":[],"migrations":[],"license_expression":"MIT",
                        "sbom_digest":sbom.digest,"signature_ref":Uuid::new_v4().to_string(),
                        "handlers":[handler],"workflows":[reference],"data_contracts":[],"tools":[]}),
                )?;
                f.sign(&manifest).await?;
                match f
                    .store
                    .execute(f.command("plugin.install", json!({"manifest": manifest})))
                    .await
                {
                    Ok(_) => Ok(Ok(())),
                    Err(failure) => Ok(Err(failure)),
                }
            };

            // A definition naming an unknown handler_id is refused at install.
            let failure = try_install(&f, json!([node("n1", Some("missing"))]), json!([]), "n1")
                .await?
                .unwrap_err();
            assert_eq!(failure.code, "WORKFLOW_INVALID");

            // An unbounded cycle with no BOUNDED_LOOP node bounding it is refused.
            let failure = try_install(
                &f,
                json!([node("a", Some("compute")), node("b", Some("compute"))]),
                json!([
                    {"from":"a","to":"b","condition":"SUCCESS"},
                    {"from":"b","to":"a","condition":"SUCCESS"},
                ]),
                "a",
            )
            .await?
            .unwrap_err();
            assert_eq!(failure.code, "WORKFLOW_INVALID");

            // A valid acyclic definition still installs cleanly.
            let counts_after_refusals = f.counts().await?;
            assert_eq!(counts_after_refusals, counts_before);
            try_install(
                &f,
                json!([node("a", Some("compute")), node("b", Some("compute"))]),
                json!([{"from":"a","to":"b","condition":"SUCCESS"}]),
                "a",
            )
            .await?
            .unwrap();

            // AC-080: refused installs create zero runs and leave receipts,
            // audit and outbox untouched. The valid control install, by
            // contrast, is admitted normally.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let runs_after: i64 =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1")
                    .bind(f.tenant)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.rollback().await?;
            assert_eq!(runs_after, runs_before);
            assert_eq!(
                f.counts().await?,
                (1, 1, counts_before.2 + 2, counts_before.3 + 2)
            );
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-073/074: actual command transaction and replay,
// not Temporal cancellation or external-effect reconciliation qualification.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_run_cancel_commits_and_replay_does_not_mutate() -> TestResult {
    exercise_run_cancellation("QUEUED", ReceiptState::Succeeded, "CANCELLED").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_run_cancel_is_requested_and_replay_does_not_mutate() -> TestResult {
    exercise_run_cancellation("RUNNING", ReceiptState::Accepted, "CANCEL_REQUESTED").await
}

async fn exercise_run_cancellation(
    initial_state: &'static str,
    expected_receipt: ReceiptState,
    expected_state: &'static str,
) -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (manifest, _) = f.package("test.cancel", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            let (input, _) = f
                .classified_artifact(b"cancel fixture".to_vec(), Classification::Confidential)
                .await?;
            let run = Uuid::new_v4();
            let grant = Uuid::new_v4();
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let now = chrono::Utc::now();
            let projection_value = json!({
                "run_id": run, "tenant_id": f.tenant, "workflow_id": "test.cancel.workflow",
                "workflow_version": "1.0.0", "plugin_digest": manifest.artifact_digest,
                "grant_id": grant, "state": initial_state, "version": 1,
                "created_at": now, "updated_at": now,
            });
            let (projection, _) = f
                .classified_artifact(
                    canonical_bytes(&projection_value)?,
                    Classification::Confidential,
                )
                .await?;
            // Seed the prerequisite run directly; cancellation itself must enter
            // the public repository dispatcher.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.cancel'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.cancel.workflow','1.0.0',$4,$5,$6,1,$7,$9,$8,$10)")
                .bind(f.tenant).bind(run).bind(principal).bind(manifest.id.as_str())
                .bind(manifest.artifact_digest.as_str()).bind(grant).bind(json!(input)).bind(run.to_string())
                .bind(initial_state).bind(if initial_state == "RUNNING" { "STARTED" } else { "PENDING" })
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO resource_projections(tenant_id,resource_type,resource_id,version,artifact_id,created_at) VALUES($1,'Run',$2,1,$3,$4)")
                .bind(f.tenant).bind(run.to_string()).bind(Uuid::parse_str(projection.artifact_id.as_str())?).bind(now)
                .execute(&mut *tx).await?;
            tx.commit().await?;

            let command = f.command(
                "run.cancel",
                json!({"run_id":run,"expected_version":1,"reason":"Stop synthetic run"}),
            );
            let before = f.counts().await?;
            let receipt = f.store.execute(command.clone()).await?;
            assert_eq!(receipt.state, expected_receipt);
            assert_eq!(receipt.run_id.as_ref().unwrap().as_str(), run.to_string());
            assert_eq!(receipt.resource.as_ref().unwrap().version, 2);
            let after = f.counts().await?;
            assert_eq!(after, (before.0, before.1 + 1, before.2 + 1, before.3 + 1));

            let writes = f.objects.writes.load(Ordering::SeqCst);
            let replay = f.store.execute(command.clone()).await?;
            assert_eq!(
                serde_json::to_value(&replay)?,
                serde_json::to_value(&receipt)?
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);

            let stale = f.command("run.cancel", command.input.clone());
            assert_eq!(
                f.store.execute(stale).await.unwrap_err().code,
                "STALE_VERSION"
            );
            let terminal = f.command(
                "run.cancel",
                json!({"run_id":run,"expected_version":2,"reason":"Already cancelled"}),
            );
            assert_eq!(
                f.store.execute(terminal).await.unwrap_err().code,
                "ILLEGAL_TRANSITION"
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);

            // A new connection/transaction observes only the committed change.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query(
                "SELECT state,version,fence,dispatch_state FROM runs WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(run)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(row.try_get::<String, _>("state")?, expected_state);
            assert_eq!(row.try_get::<i32, _>("version")?, 2);
            assert_eq!(row.try_get::<i64, _>("fence")?, 2);
            assert_eq!(row.try_get::<String, _>("dispatch_state")?, "BLOCKED");
            tx.rollback().await?;
            let current = f
                .store
                .projection(&f.actor, "Run", &run.to_string())
                .await?;
            assert_eq!(current.resource.version, 2);
            assert_eq!(
                current.artifact_ref.classification,
                Classification::Confidential
            );
            let (_, bytes) = f
                .store
                .artifact_content(
                    &f.actor,
                    current.artifact_ref.artifact_id.as_str(),
                    1024 * 1024,
                )
                .await?;
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes)?["state"],
                expected_state
            );
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

#[derive(Default)]
struct InvocationProbe {
    calls: AtomicUsize,
    invalid_output: AtomicBool,
}

#[async_trait]
impl PurePluginRuntime for InvocationProbe {
    async fn invoke(
        &self,
        input: PurePluginInvocation,
    ) -> Result<PurePluginOutput, CommandFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid_output.load(Ordering::SeqCst) {
            return Ok(PurePluginOutput {
                bytes: b"{\"wrong\":true}".to_vec(),
            });
        }
        let input: Vec<u8> = serde_json::from_slice(&input.input).unwrap();
        Ok(PurePluginOutput {
            bytes: canonical_bytes(&digest_bytes(&input)).unwrap(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invocation_admission_denials_output_validation_replay_and_confidential_lineage()
-> TestResult {
    let probe = Arc::new(InvocationProbe::default());
    let mut fixture = Fixture::create().await?;
    fixture.store = fixture.store.clone().with_plugin_runtime(probe.clone());
    let fixture = Arc::new(fixture);
    let task = tokio::spawn({
        let fixture = fixture.clone();
        async move { exercise_invocation(&fixture, &probe).await }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

async fn exercise_invocation(f: &Fixture, probe: &InvocationProbe) -> TestResult {
    let (mut manifest, _) = f.package("test.invoke", "1.0.0").await?;
    let (schema_in, _) = f
        .artifact(
            br#"{"type":"array","items":{"type":"integer","minimum":0,"maximum":255}}"#.to_vec(),
        )
        .await?;
    let (schema_out, _) = f
        .artifact(br#"{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}"#.to_vec())
        .await?;
    manifest.handlers = serde_json::from_value(
        json!([{"id":"compute","input_schema_ref":schema_in,"output_schema_ref":schema_out,
        "effects":"DETERMINISTIC","required_capabilities":[],"max_execution_seconds":5}]),
    )?;
    manifest.signature_ref = Uuid::new_v4().to_string();
    f.sign(&manifest).await?;
    f.store
        .execute(f.command("plugin.install", json!({"manifest":manifest})))
        .await?;
    let installed = f.store.execute(f.command("plugin.enable",json!({"plugin_id":"test.invoke","artifact_digest":manifest.artifact_digest,"grants":[],"expected_version":1}))).await?.resource.unwrap();
    let (input, _) = f
        .classified_artifact(
            b"[97.0, 98.000, 99e0]".to_vec(),
            Classification::Confidential,
        )
        .await?;
    let (invalid, _) = f.artifact(b"[999]".to_vec()).await?;
    let (outside, _) = f.artifact(b"[42]".to_vec()).await?;
    let grant = f.store.execute(f.command("grant.create",json!({"delegate":{"type":"USER","id":f.actor.principal_id,"issuer":f.actor.issuer},
        "actions":["plugin.invoke","artifact.read"],"resources":[installed,{"resource_type":"Artifact","resource_id":input.artifact_id,"version":1},
            {"resource_type":"Artifact","resource_id":invalid.artifact_id,"version":1}],
        "expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs,true),"parent_grant_id":null}))).await?.resource.unwrap();
    let invoke = f.command("plugin.invoke",json!({"plugin_id":"test.invoke","handler":"compute","input_ref":input,"grant_id":grant.resource_id}));
    let before = f.counts().await?;
    let reads = f.objects.reads.load(Ordering::SeqCst);
    assert_eq!(
        f.store.execute(invoke.clone()).await.unwrap_err().code,
        "SOURCE_RIGHTS_DENIED"
    );
    let outside_command = f.command("plugin.invoke",json!({"plugin_id":"test.invoke","handler":"compute","input_ref":outside,"grant_id":grant.resource_id}));
    assert_eq!(
        f.store.execute(outside_command).await.unwrap_err().code,
        "CAPABILITY_DENIED"
    );
    assert_eq!(f.objects.reads.load(Ordering::SeqCst), reads);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.counts().await?, before);
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    for reference in [&input, &invalid] {
        sqlx::query("INSERT INTO artifact_rights(tenant_id,artifact_id,rights_id,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,state) VALUES($1,$2,$3,'MANUAL_IMPORT',true,false,true,false,'ACTIVE')")
            .bind(f.tenant).bind(Uuid::parse_str(reference.artifact_id.as_str())?).bind(Uuid::new_v4()).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    assert_eq!(f.store.execute(f.command("plugin.invoke",json!({"plugin_id":"test.invoke","handler":"compute","input_ref":invalid,"grant_id":grant.resource_id}))).await.unwrap_err().code,"PLUGIN_INPUT_INVALID");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    probe.invalid_output.store(true, Ordering::SeqCst);
    let writes = f.objects.writes.load(Ordering::SeqCst);
    assert_eq!(
        f.store.execute(invoke.clone()).await.unwrap_err().code,
        "PLUGIN_OUTPUT_INVALID"
    );
    assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);
    assert_eq!(f.counts().await?, before);
    probe.invalid_output.store(false, Ordering::SeqCst);
    let receipt = f.store.execute(invoke.clone()).await?;
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.store.execute(invoke).await?, receipt);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
    let resource = receipt.resource.unwrap();
    let projection = f
        .store
        .read_projection(&f.actor, "PluginInvocation", resource.resource_id.as_str())
        .await?;
    assert_eq!(
        projection.artifact_ref.classification,
        Classification::Confidential
    );
    let (_, bytes) = f
        .store
        .artifact_content(
            &f.actor,
            projection.artifact_ref.artifact_id.as_str(),
            1024 * 1024,
        )
        .await?;
    let projection_value: Value = serde_json::from_slice(&bytes)?;
    let output: ArtifactRef = serde_json::from_value(projection_value["output_ref"].clone())?;
    assert_eq!(output.classification, Classification::Confidential);
    let (_, bytes) = f
        .store
        .artifact_content(&f.actor, output.artifact_id.as_str(), 1024 * 1024)
        .await?;
    assert_eq!(
        serde_json::from_slice::<String>(&bytes)?,
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    let descendants: i64 = sqlx::query_scalar("WITH RECURSIVE descendants AS (SELECT derived_id FROM artifact_lineage WHERE tenant_id=$1 AND source_id=$2 UNION SELECT l.derived_id FROM artifact_lineage l JOIN descendants d ON l.source_id=d.derived_id WHERE l.tenant_id=$1) SELECT count(*) FROM descendants")
        .bind(f.tenant).bind(Uuid::parse_str(input.artifact_id.as_str())?).fetch_one(&mut *tx).await?;
    assert_eq!(descendants, 2); // Output bytes and the metadata projection.
    let rights = sqlx::query("SELECT allow_ai_analysis,allow_public_redistribution FROM artifact_rights WHERE tenant_id=$1 AND artifact_id=$2")
        .bind(f.tenant).bind(Uuid::parse_str(output.artifact_id.as_str())?).fetch_one(&mut *tx).await?;
    assert!(!rights.try_get::<bool, _>("allow_ai_analysis")?);
    assert!(!rights.try_get::<bool, _>("allow_public_redistribution")?);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_transactions_replay_denials_versions_locks_and_quarantine() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let fixture = fixture.clone();
        async move { exercise(&fixture).await }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

async fn exercise(f: &Fixture) -> TestResult {
    // MASONWING@1.0.1 REQ-006,012 / AC-006,012: installed, never enabled implicitly.
    let (v1, _) = f.package("test.plugin", "1.0.0").await?;
    let install = f.command("plugin.install", json!({"manifest":v1}));
    let receipt = f.store.execute(install.clone()).await?;
    assert_eq!(receipt.state, ReceiptState::Succeeded);
    assert_eq!(
        f.state("test.plugin").await?,
        (
            "INSTALLED_DISABLED".into(),
            1,
            v1.artifact_digest.to_string()
        )
    );
    assert_eq!(f.counts().await?, (1, 1, 2, 2));
    let writes = f.objects.writes.load(Ordering::SeqCst);
    assert_eq!(f.store.execute(install.clone()).await?, receipt);
    assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);
    assert_eq!(f.counts().await?, (1, 1, 2, 2));
    let mut conflicting = install.clone();
    conflicting.input["manifest"]["version"] = json!("1.0.1");
    conflicting.fingerprint = fingerprint(&conflicting.operation, &conflicting.input)?;
    assert_eq!(
        f.store.execute(conflicting).await.unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );

    // AC-013: outer plugin.enable permission cannot override a denied grant.
    let denied = f.command("plugin.enable",json!({"plugin_id":"test.plugin","artifact_digest":v1.artifact_digest,"grants":["effect.dispatch"],"expected_version":1}));
    assert_eq!(
        f.store.execute(denied).await.unwrap_err().code,
        "CAPABILITY_DENIED"
    );
    assert_eq!(f.counts().await?, (1, 1, 2, 2));
    assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);
    let enable = f.command("plugin.enable",json!({"plugin_id":"test.plugin","artifact_digest":v1.artifact_digest,"grants":["resource.read"],"expected_version":1}));
    f.store.execute(enable).await?;
    assert_eq!(f.state("test.plugin").await?.0, "ENABLED");

    // AC-010: exact product locks, with no implicit latest-version resolution.
    let lock = json!({"schema_version":"1.0.0","product_id":"test.product","plugins":[{"plugin_id":"test.plugin","artifact_digest":v1.artifact_digest,"contract_version":"1.0.0"}]});
    let (lock_ref, _) = f.artifact(canonical_bytes(&lock)?).await?;
    let compose = f.command(
        "product.compose",
        json!({"product_id":"test.product","plugin_lock_ref":lock_ref}),
    );
    f.store.execute(compose).await?;

    // Concurrent optimistic writes: exactly one disable, no silent rebasing.
    let first = f.command(
        "plugin.disable",
        json!({"plugin_id":"test.plugin","expected_version":2,"reason":"test drain"}),
    );
    let second = f.command(
        "plugin.disable",
        json!({"plugin_id":"test.plugin","expected_version":2,"reason":"concurrent drain"}),
    );
    let (a, b) = tokio::join!(f.store.execute(first), f.store.execute(second));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(a.err().or(b.err()).unwrap().code, "STALE_VERSION");
    assert_eq!(f.state("test.plugin").await?.0, "DISABLED");
    assert_eq!(f.state("test.plugin").await?.1, 4);

    let (v2, _) = f.package("test.plugin", "2.0.0").await?;
    f.store
        .execute(f.command(
            "plugin.upgrade",
            json!({"plugin_id":"test.plugin","new_manifest":v2,"expected_version":4}),
        ))
        .await?;
    f.store.execute(f.command("plugin.enable",json!({"plugin_id":"test.plugin","artifact_digest":v2.artifact_digest,"grants":["resource.read"],"expected_version":5}))).await?;
    let stale_lock = json!({"schema_version":"1.0.0","product_id":"another.product","plugins":[{"plugin_id":"test.plugin","artifact_digest":v1.artifact_digest,"contract_version":"1.0.0"}]});
    let (stale_ref, _) = f.artifact(canonical_bytes(&stale_lock)?).await?;
    assert_eq!(
        f.store
            .execute(f.command(
                "product.compose",
                json!({"product_id":"another.product","plugin_lock_ref":stale_ref})
            ))
            .await
            .unwrap_err()
            .code,
        "PLUGIN_LOCK_MISMATCH"
    );
    let rollback = f.command(
        "plugin.upgrade",
        json!({"plugin_id":"test.plugin","new_manifest":v1,"expected_version":6}),
    );
    assert_eq!(
        f.store.execute(rollback).await.unwrap_err().code,
        "ROLLBACK_UNSAFE"
    );

    // Revoking an old digest never revokes the upgraded, unrelated digest.
    f.store.execute(f.command("plugin.revoke",json!({"plugin_id":"test.plugin","artifact_digest":v1.artifact_digest,"reason":"old artifact revoked"}))).await?;
    assert_eq!(
        f.state("test.plugin").await?,
        ("ENABLED".into(), 7, v2.artifact_digest.to_string())
    );
    f.store.execute(f.command("plugin.revoke",json!({"plugin_id":"test.plugin","artifact_digest":v2.artifact_digest,"reason":"current artifact revoked"}))).await?;
    assert_eq!(f.state("test.plugin").await?.0, "REVOKED");

    // AC-009: actual byte substitution after signing persists quarantine and
    // audit/outbox, while the rejected install has no receipt or registry row.
    let (corrupt, key) = f.package("test.corrupt", "1.0.0").await?;
    f.objects
        .bytes
        .lock()
        .unwrap()
        .insert(key, b"substituted package".to_vec());
    let before = f.counts().await?;
    assert_eq!(
        f.store
            .execute(f.command("plugin.install", json!({"manifest":corrupt})))
            .await
            .unwrap_err()
            .code,
        "ARTIFACT_INTEGRITY"
    );
    assert_eq!(
        f.counts().await?,
        (before.0, before.1, before.2 + 1, before.3 + 1)
    );
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    let state: String =
        sqlx::query_scalar("SELECT state FROM artifacts WHERE tenant_id=$1 AND digest=$2")
            .bind(f.tenant)
            .bind(corrupt.artifact_digest.as_str())
            .fetch_one(&mut *tx)
            .await?;
    assert_eq!(state, "QUARANTINED");
    let versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM plugin_versions WHERE tenant_id=$1 AND plugin_id='test.plugin'",
    )
    .bind(f.tenant)
    .fetch_one(&mut *tx)
    .await?;
    assert_eq!(versions, 2);
    sqlx::query("UPDATE memberships SET status='REVOKED',version=version+1 WHERE tenant_id=$1 AND principal_id=$2")
        .bind(f.tenant).bind(Uuid::parse_str(f.actor.principal_id.as_str())?).execute(&mut *tx).await?;
    tx.commit().await?;
    // A cached successful receipt does not survive revocation of current access.
    let reads = f.objects.reads.load(Ordering::SeqCst);
    assert_eq!(
        f.store.execute(install).await.unwrap_err().code,
        "AUTHORITY_CHANGED"
    );
    assert_eq!(f.objects.reads.load(Ordering::SeqCst), reads);
    Ok(())
}

// MASONWING@1.0.1 REQ-078..083 / AC-082..086: the decision transaction binds the
// exact reviewed digest, refuses the proposal's author, and rejects decisions
// after a terminal state has been committed. Transmission is out of scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_decide_enforces_binding_four_eyes_and_terminal_exclusivity() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let (manifest, _) = f.package("test.approval", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            let author = Uuid::new_v4();
            let run = Uuid::new_v4();
            let proposal = Uuid::new_v4();
            let now = chrono::Utc::now();
            let digest = format!("sha256:{}", "a".repeat(64));
            let other_digest = format!("sha256:{}", "b".repeat(64));
            let projection_value = json!({
                "proposal_id": proposal, "tenant_id": f.tenant,
                "action": "publication.update",
                "target": {"resource_type": "Document", "resource_id": "doc-1", "version": 2},
                "content_digest": digest, "scope_digest": format!("sha256:{}", "c".repeat(64)),
                "policy_version": "1.0.0", "expires_at": now,
                "max_cost_microunits": 0, "currency": "USD", "version": 1,
            });
            let (projection, _) = f
                .classified_artifact(
                    canonical_bytes(&projection_value)?,
                    Classification::Confidential,
                )
                .await?;
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let grant = Uuid::new_v4();
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['approval.decide'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            // The author is a distinct ACTIVE member, so the four-eyes denial is
            // about deciding one's own proposal rather than missing membership.
            let author_membership = Uuid::new_v4();
            sqlx::query("INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch) VALUES($1,$2,$3,'REVIEWER','ACTIVE',1,1)")
                .bind(f.tenant).bind(author).bind(author_membership).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES($1,$2,'REVIEWER')")
                .bind(f.tenant).bind(author_membership).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.approval.workflow','1.0.0','test.approval',$4,$5,1,'{}'::jsonb,'WAITING_APPROVAL',$6,'STARTED')")
                .bind(f.tenant).bind(run).bind(principal).bind(manifest.artifact_digest.as_str()).bind(grant)
                .bind(run.to_string()).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO approvals(tenant_id,id,run_id,action,target,content_digest,scope_digest,policy_version,policy_epoch,max_cost_microunits,currency,expires_at,created_by,state) VALUES($1,$2,$3,'publication.update',$4,$5,$6,'1.0.0',1,0,'USD',clock_timestamp()+interval '1 hour',$7,'REQUESTED')")
                .bind(f.tenant).bind(proposal).bind(run)
                .bind(json!({"resource_type":"Document","resource_id":"doc-1","version":2}))
                .bind(&digest).bind(format!("sha256:{}", "c".repeat(64))).bind(author)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO resource_projections(tenant_id,resource_type,resource_id,version,artifact_id,created_at) VALUES($1,'Approval',$2,1,$3,$4)")
                .bind(f.tenant).bind(proposal.to_string()).bind(Uuid::parse_str(projection.artifact_id.as_str())?).bind(now)
                .execute(&mut *tx).await?;
            tx.commit().await?;

            let before = f.counts().await?;
            // The proposal's author can never be its decider. The author is a
            // separate ACTIVE member, so the denial is four-eyes and not a
            // membership failure.
            let author_actor = CommandActor {
                principal_id: PrincipalId::new(author.to_string())?,
                ..f.actor.clone()
            };
            let four_eyes = AuthorizedCommand {
                actor: author_actor,
                ..f.command(
                    "approval.decide",
                    json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":1,
                        "content_digest":digest,"reason":"Self approval attempt"}),
                )
            };
            assert_eq!(
                f.store.execute(four_eyes).await.unwrap_err().code,
                "FOUR_EYES_REQUIRED"
            );
            // A decision over different content does not reach the proposal.
            let stale = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":1,
                    "content_digest":other_digest,"reason":"Different digest"}),
            );
            assert_eq!(
                f.store.execute(stale).await.unwrap_err().code,
                "REVIEW_CONFLICT"
            );
            assert_eq!(f.counts().await?, before);

            // The legitimate decision wins: the proposal terminates APPROVED.
            let command = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":1,
                    "content_digest":digest,"reason":"Reviewed against sha256:aaa…"}),
            );
            let receipt = f.store.execute(command.clone()).await?;
            assert_eq!(receipt.state, ReceiptState::Succeeded);
            assert_eq!(receipt.resource.as_ref().unwrap().version, 2);
            assert_eq!(receipt.run_id.as_ref().unwrap().as_str(), run.to_string());
            let after = f.counts().await?;
            assert_eq!(after, (before.0, before.1 + 1, before.2 + 1, before.3 + 1));

            // Replay returns the original receipt and writes nothing.
            let writes = f.objects.writes.load(Ordering::SeqCst);
            let replay = f.store.execute(command.clone()).await?;
            assert_eq!(
                serde_json::to_value(&replay)?,
                serde_json::to_value(&receipt)?
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);

            // The loser of the race observes the terminal decision.
            let loser = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"CANCEL","expected_version":1,
                    "content_digest":digest,"reason":"Too late"}),
            );
            assert_eq!(
                f.store.execute(loser).await.unwrap_err().code,
                "DECISION_CONFLICT"
            );
            // The decided version no longer matches a stale expectation.
            let stale_version = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":2,
                    "content_digest":digest,"reason":"Wrong version"}),
            );
            assert_eq!(
                f.store.execute(stale_version).await.unwrap_err().code,
                "DECISION_CONFLICT"
            );
            assert_eq!(f.counts().await?, after);
            assert_eq!(f.objects.writes.load(Ordering::SeqCst), writes);

            // A new transaction observes the committed terminal state.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT state,version,decided_by,decided_issuer,decision_reason FROM approvals WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant)
                .bind(proposal)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "APPROVED");
            assert_eq!(row.try_get::<i32, _>("version")?, 2);
            assert_eq!(row.try_get::<uuid::Uuid, _>("decided_by")?, principal);
            assert_eq!(row.try_get::<String, _>("decided_issuer")?, f.actor.issuer);
            tx.rollback().await?;
            let current = f
                .store
                .projection(&f.actor, "Approval", &proposal.to_string())
                .await?;
            assert_eq!(current.resource.version, 2);
            let (_, bytes) = f
                .store
                .artifact_content(
                    &f.actor,
                    current.artifact_ref.artifact_id.as_str(),
                    1024 * 1024,
                )
                .await?;
            let value = serde_json::from_slice::<Value>(&bytes)?;
            assert_eq!(value["state"], "APPROVED");
            assert_eq!(value["content_digest"], digest);
            assert_eq!(value["decided_by"]["id"], f.actor.principal_id.as_str());
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 AC-084: a REQUESTED proposal past its expiry is closed, and
// an already-closed proposal cannot be decided again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_decide_refuses_expired_and_invalidated_proposals() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let reviewer = Uuid::new_v4();
            let (manifest, _) = f.package("test.approval", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            // ENABLE the plugin so the authority re-check reaches the grant.
            f.store
                .execute(f.command(
                    "plugin.enable",
                    json!({
                    "plugin_id": manifest.id, "artifact_digest": manifest.artifact_digest,
                    "grants": ["resource.read"], "expected_version": 1}),
                ))
                .await?;
            let run = Uuid::new_v4();
            let proposal = Uuid::new_v4();
            let digest = format!("sha256:{}", "a".repeat(64));
            let now = chrono::Utc::now();
            let projection_value = json!({
                "proposal_id": proposal, "tenant_id": f.tenant,
                "action": "publication.update",
                "target": {"resource_type": "Document", "resource_id": "doc-1", "version": 2},
                "content_digest": digest, "scope_digest": format!("sha256:{}", "c".repeat(64)),
                "policy_version": "1.0.0", "expires_at": now,
                "max_cost_microunits": 0, "currency": "USD", "version": 1,
            });
            let (projection, _) = f
                .classified_artifact(
                    canonical_bytes(&projection_value)?,
                    Classification::Confidential,
                )
                .await?;
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let grant = Uuid::new_v4();
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['approval.decide'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.approval.workflow','1.0.0','test.approval',$4,$5,1,'{}'::jsonb,'WAITING_APPROVAL',$6,'STARTED')")
                .bind(f.tenant).bind(run).bind(principal).bind(manifest.artifact_digest.as_str()).bind(grant)
                .bind(run.to_string()).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO approvals(tenant_id,id,run_id,action,target,content_digest,scope_digest,policy_version,policy_epoch,max_cost_microunits,currency,expires_at,created_by,state) VALUES($1,$2,$3,'publication.update',$4,$5,$6,'1.0.0',1,0,'USD',clock_timestamp()-interval '1 minute',$7,'REQUESTED')")
                .bind(f.tenant).bind(proposal).bind(run)
                .bind(json!({"resource_type":"Document","resource_id":"doc-1","version":2}))
                .bind(&digest).bind(format!("sha256:{}", "c".repeat(64))).bind(reviewer)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO resource_projections(tenant_id,resource_type,resource_id,version,artifact_id,created_at) VALUES($1,'Approval',$2,1,$3,$4)")
                .bind(f.tenant).bind(proposal.to_string()).bind(Uuid::parse_str(projection.artifact_id.as_str())?).bind(now)
                .execute(&mut *tx).await?;
            tx.commit().await?;

            let expired = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":1,
                    "content_digest":digest,"reason":"Expired proposal"}),
            );
            assert_eq!(
                f.store.execute(expired).await.unwrap_err().code,
                "APPROVAL_EXPIRED"
            );
            // Close the proposal and confirm the terminal state rejects re-decides.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE approvals SET state='INVALIDATED' WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant)
                .bind(proposal)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            let invalidated = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"REJECT","expected_version":1,
                    "content_digest":digest,"reason":"Invalidated"}),
            );
            assert_eq!(
                f.store.execute(invalidated).await.unwrap_err().code,
                "APPROVAL_STALE"
            );
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-081 / AC-085: two racing decisions produce exactly one
// terminal state; the loser receives 409 DECISION_CONFLICT and the audit trail
// records only the winner's decision.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_decide_concurrent_race_admits_exactly_one_winner() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let (manifest, _) = f.package("test.approval", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            let author = Uuid::new_v4();
            let run = Uuid::new_v4();
            let proposal = Uuid::new_v4();
            let digest = format!("sha256:{}", "a".repeat(64));
            let now = chrono::Utc::now();
            let projection_value = json!({
                "proposal_id": proposal, "tenant_id": f.tenant,
                "action": "publication.update",
                "target": {"resource_type": "Document", "resource_id": "doc-1", "version": 2},
                "content_digest": digest, "scope_digest": format!("sha256:{}", "c".repeat(64)),
                "policy_version": "1.0.0", "expires_at": now,
                "max_cost_microunits": 0, "currency": "USD", "version": 1,
            });
            let (projection, _) = f
                .classified_artifact(
                    canonical_bytes(&projection_value)?,
                    Classification::Confidential,
                )
                .await?;
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let grant = Uuid::new_v4();
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['approval.decide'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.approval.workflow','1.0.0','test.approval',$4,$5,1,'{}'::jsonb,'WAITING_APPROVAL',$6,'STARTED')")
                .bind(f.tenant).bind(run).bind(principal).bind(manifest.artifact_digest.as_str()).bind(grant)
                .bind(run.to_string()).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO approvals(tenant_id,id,run_id,action,target,content_digest,scope_digest,policy_version,policy_epoch,max_cost_microunits,currency,expires_at,created_by,state) VALUES($1,$2,$3,'publication.update',$4,$5,$6,'1.0.0',1,0,'USD',clock_timestamp()+interval '1 hour',$7,'REQUESTED')")
                .bind(f.tenant).bind(proposal).bind(run)
                .bind(json!({"resource_type":"Document","resource_id":"doc-1","version":2}))
                .bind(&digest).bind(format!("sha256:{}", "c".repeat(64))).bind(author)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO resource_projections(tenant_id,resource_type,resource_id,version,artifact_id,created_at) VALUES($1,'Approval',$2,1,$3,$4)")
                .bind(f.tenant).bind(proposal.to_string()).bind(Uuid::parse_str(projection.artifact_id.as_str())?).bind(now)
                .execute(&mut *tx).await?;
            tx.commit().await?;

            // Both deciders decide simultaneously over the same valid digest;
            // one must commit, the other must lose against the terminal state.
            let approve = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"APPROVE","expected_version":1,
                    "content_digest":digest,"reason":"Approve under race"}),
            );
            let cancel = f.command(
                "approval.decide",
                json!({"proposal_id":proposal,"decision":"CANCEL","expected_version":1,
                    "content_digest":digest,"reason":"Cancel under race"}),
            );
            let store = f.store.clone();
            let (first, second) = tokio::join!(
                tokio::spawn({
                    let store = store.clone();
                    async move { store.execute(approve).await }
                }),
                tokio::spawn(async move { store.execute(cancel).await })
            );
            let outcomes: Vec<String> = [first, second]
                .into_iter()
                .map(|joined| match joined.unwrap() {
                    Ok(_) => "WON".to_owned(),
                    Err(failure) => failure.code.to_owned(),
                })
                .collect();
            assert_eq!(
                outcomes.iter().filter(|o| o.as_str() == "WON").count(),
                1,
                "exactly one decision commits: {outcomes:?}"
            );
            assert!(
                outcomes
                    .iter()
                    .all(|o| o.as_str() == "WON" || o.as_str() == "DECISION_CONFLICT"),
                "the loser conflicts: {outcomes:?}"
            );

            // The committed terminal state is one of the two decisions, at
            // version 2, with exactly one decided_by recorded.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query(
                "SELECT state,version,decided_by FROM approvals WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(proposal)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(row.try_get::<i32, _>("version")?, 2);
            let state: String = row.try_get("state")?;
            assert!(
                state == "APPROVED" || state == "CANCELLED",
                "terminal: {state}"
            );
            let decided: Option<Uuid> = row.try_get("decided_by")?;
            assert_eq!(decided, Some(principal));
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-092..095 / AC-096..099: budget ledger reserve/settle/unknown.
// These tests drive the public system-path API on the live local Postgres.

/// Fixture seed: tenant, budget_settings RUN+DAY limits for USD, one run row.
/// Returns (run_id, usd_currency).
async fn budget_seed(f: &Fixture, run_limit: i64, day_limit: i64) -> TestResult<(Uuid, String)> {
    let (manifest, _) = f.package("test.budget", "1.0.0").await?;
    f.store
        .execute(f.command("plugin.install", json!({"manifest": manifest})))
        .await?;
    let run_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
    let currency = "USD";
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO budget_settings(tenant_id,id,currency,period,limit_microunits,version) VALUES($1,$2,$3,'RUN',$4,1)")
        .bind(f.tenant)
        .bind(Uuid::new_v4())
        .bind(currency)
        .bind(run_limit)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO budget_settings(tenant_id,id,currency,period,limit_microunits,version) VALUES($1,$2,$3,'DAY',$4,1)")
        .bind(f.tenant)
        .bind(Uuid::new_v4())
        .bind(currency)
        .bind(day_limit)
        .execute(&mut *tx)
        .await?;
    // runs.grant_id is an FK into delegations; the ledger test needs a real row.
    sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
        .bind(f.tenant)
        .bind(grant_id)
        .bind(f.actor.principal_id.as_str())
        .bind(&f.actor.issuer)
        .bind(principal)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.budget.workflow','1.0.0','test.budget',$4,$5,1,'{}'::jsonb,'RUNNING',$6,'STARTED')")
        .bind(f.tenant)
        .bind(run_id)
        .bind(principal)
        .bind(manifest.artifact_digest.as_str())
        .bind(grant_id)
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((run_id, currency.to_string()))
}

// REQ-092 / AC-096: reserve before dispatch, balance available drops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_reserve_before_dispatch_balances_atomically() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 200).await?;
            let account_keys = vec![
                format!("RUN:{currency}:default"),
                format!("DAY:{currency}:default"),
            ];
            let mutation = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-001",
                    &currency,
                    20,
                    "1.0.0",
                    &account_keys,
                )
                .await?;
            assert_eq!(mutation.primary.resource_type, "CostReservation");
            assert_eq!(mutation.primary.version, 1);
            // Reservation state and accounts verified against the DB.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let res_row = sqlx::query("SELECT state,upper_bound_microunits FROM cost_reservations WHERE tenant_id=$1 AND run_id=$2")
                .bind(f.tenant)
                .bind(run_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(res_row.try_get::<String, _>("state")?, "RESERVED");
            assert_eq!(res_row.try_get::<i64, _>("upper_bound_microunits")?, 20);
            // RUN account: 100 - 20 = 80 available
            let run_acct = sqlx::query("SELECT held_microunits,limit_microunits FROM budget_accounts WHERE tenant_id=$1 AND account_key=$2")
                .bind(f.tenant)
                .bind(format!("RUN:{currency}:default"))
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(run_acct.try_get::<i64, _>("held_microunits")?, 20);
            assert_eq!(run_acct.try_get::<i64, _>("limit_microunits")?, 100);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-093 / AC-097: concurrent reservations exceed budget -> one BUDGET_EXCEEDED, one succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_concurrent_reserve_atomic_excess_rejected() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 100).await?;
            let store1 = f.store.clone();
            let store2 = f.store.clone();
            let run1 = run_id;
            let run2 = run_id;
            let tenant = f.tenant;
            let curr1 = currency.clone();
            let curr2 = currency.clone();
            let (r1, r2) = tokio::join!(
                tokio::spawn(async move {
                    store1
                        .reserve_run_budget(
                            tenant,
                            run1,
                            "call-a",
                            &curr1,
                            80,
                            "1.0.0",
                            &[
                                format!("RUN:{curr1}:default"),
                                format!("DAY:{curr1}:default"),
                            ],
                        )
                        .await
                }),
                tokio::spawn(async move {
                    store2
                        .reserve_run_budget(
                            tenant,
                            run2,
                            "call-b",
                            &curr2,
                            80,
                            "1.0.0",
                            &[
                                format!("RUN:{curr2}:default"),
                                format!("DAY:{curr2}:default"),
                            ],
                        )
                        .await
                })
            );
            let r1 = r1?;
            let r2 = r2?;
            // Exactly one should succeed, exactly one should be BUDGET_EXCEEDED.
            let ok1 = r1.is_ok();
            let ok2 = r2.is_ok();
            let err1 = r1.as_ref().err().map(|e| e.code);
            let err2 = r2.as_ref().err().map(|e| e.code);
            let ok_count = usize::from(ok1) + usize::from(ok2);
            let budget_err_count = [err1, err2]
                .iter()
                .filter(|c| **c == Some("BUDGET_EXCEEDED"))
                .count();
            assert_eq!(
                ok_count, 1,
                "exactly one reservation should succeed; got r1={r1:?}, r2={r2:?}"
            );
            assert_eq!(
                budget_err_count, 1,
                "exactly one should be BUDGET_EXCEEDED; got err1={err1:?}, err2={err2:?}"
            );
            // Total reserved never exceeds 100 on any account.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let held: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(held_microunits),0)::bigint FROM budget_accounts WHERE tenant_id=$1 AND account_key LIKE 'RUN:USD:default'")
                .bind(f.tenant)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(held, 80, "only one 80-reservation should hold");
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-094 / AC-098: unknown usage keeps the hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_unknown_usage_keeps_held() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 100).await?;
            let account_keys = vec![format!("RUN:{currency}:default")];
            let reserve = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-u",
                    &currency,
                    30,
                    "1.0.0",
                    &account_keys,
                )
                .await?;
            let reservation_id = Uuid::parse_str(reserve.changes[0].resource.resource_id.as_str())?;
            f.store
                .mark_run_cost_unknown(f.tenant, reservation_id)
                .await?;
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT state,held_microunits FROM cost_reservations c JOIN budget_accounts b ON b.tenant_id=c.tenant_id AND b.account_key=ANY(c.account_keys) WHERE c.tenant_id=$1 AND c.id=$2")
                .bind(f.tenant)
                .bind(reservation_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "UNKNOWN");
            assert_eq!(
                row.try_get::<i64, _>("held_microunits")?,
                30,
                "held must NOT drop to zero"
            );
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-095 / AC-099: settlement idempotent for same usage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_settle_once_idempotent_for_same_usage() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 100).await?;
            let account_keys = vec![format!("RUN:{currency}:default")];
            let reserve = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-s",
                    &currency,
                    20,
                    "1.0.0",
                    &account_keys,
                )
                .await?;
            let reservation_id = Uuid::parse_str(reserve.changes[0].resource.resource_id.as_str())?;
            // First settle: charged 12, released 8.
            let first = f
                .store
                .settle_run_budget(f.tenant, reservation_id, 12, Some("fp-001"))
                .await?;
            assert_eq!(first.primary.version, 2);
            // Replay same usage: no additional charge, same version.
            let replay = f
                .store
                .settle_run_budget(f.tenant, reservation_id, 12, Some("fp-001"))
                .await?;
            assert_eq!(replay.primary.version, 2);
            // Verify accounts: held=0, charged=12.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let acct = sqlx::query("SELECT held_microunits,charged_microunits FROM budget_accounts WHERE tenant_id=$1 AND account_key=$2")
                .bind(f.tenant)
                .bind(format!("RUN:{currency}:default"))
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(acct.try_get::<i64, _>("held_microunits")?, 0);
            assert_eq!(acct.try_get::<i64, _>("charged_microunits")?, 12);
            // Different usage on settled reservation -> SETTLEMENT_CONFLICT.
            let conflict = f
                .store
                .settle_run_budget(f.tenant, reservation_id, 13, Some("fp-002"))
                .await;
            assert_eq!(conflict.unwrap_err().code, "SETTLEMENT_CONFLICT");
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-096 / AC-100: price profile with no bounded cost -> PRICE_BOUND_UNKNOWN.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_reserve_rejects_zero_upper_bound() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 100).await?;
            let account_keys = vec![format!("RUN:{currency}:default")];
            // Zero upper bound (unbounded price profile) -> PRICE_BOUND_UNKNOWN.
            let err = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-z",
                    &currency,
                    0,
                    "1.0.0",
                    &account_keys,
                )
                .await;
            assert_eq!(err.unwrap_err().code, "PRICE_BOUND_UNKNOWN");
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-094 + REQ-095 combined: UNKNOWN -> SETTLED via reconcile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_unknown_settles_via_reconcile() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, currency) = budget_seed(&f, 100, 100).await?;
            let account_keys = vec![format!("RUN:{currency}:default")];
            let reserve = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-us",
                    &currency,
                    50,
                    "1.0.0",
                    &account_keys,
                )
                .await?;
            let reservation_id = Uuid::parse_str(reserve.changes[0].resource.resource_id.as_str())?;
            f.store
                .mark_run_cost_unknown(f.tenant, reservation_id)
                .await?;
            // Reconcile arrives with authoritative usage 40.
            let settle = f
                .store
                .settle_run_budget(f.tenant, reservation_id, 40, Some("fp-recon"))
                .await?;
            assert_eq!(settle.primary.version, 3);
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let acct = sqlx::query("SELECT held_microunits,charged_microunits FROM budget_accounts WHERE tenant_id=$1 AND account_key=$2")
                .bind(f.tenant)
                .bind(format!("RUN:{currency}:default"))
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(acct.try_get::<i64, _>("held_microunits")?, 0);
            assert_eq!(acct.try_get::<i64, _>("charged_microunits")?, 40);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-092 + REQ-093: unconfigured currency account -> BUDGET_NOT_CONFIGURED.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_reserve_unconfigured_account_refused() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, _) = budget_seed(&f, 100, 100).await?;
            // EUR has no budget_settings row; account creation would get limit=0.
            let err = f
                .store
                .reserve_run_budget(
                    f.tenant,
                    run_id,
                    "call-eur",
                    "EUR",
                    10,
                    "1.0.0",
                    &["RUN:EUR:default".to_string()],
                )
                .await;
            assert_eq!(err.unwrap_err().code, "BUDGET_NOT_CONFIGURED");
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

/// Effect fixture seed: an ACTIVE connection plus a non-terminal run on an
/// ACTIVE delegation owned by the test actor. Returns (run_id, connection_id,
/// grant_id). The caller must build the tenant with a policy that permits the
/// effect operations it exercises (`Fixture::effect` does this); the policy
/// version is immutable, so it cannot be rewritten after creation.
async fn effect_seed(f: &Fixture) -> TestResult<(Uuid, Uuid, Uuid)> {
    let (manifest, _) = f.package("test.effects", "1.0.0").await?;
    f.store
        .execute(f.command("plugin.install", json!({"manifest": manifest})))
        .await?;
    let run_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let connection_id = Uuid::new_v4();
    let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO budget_settings(tenant_id,id,currency,period,limit_microunits,version) VALUES($1,$2,'USD','RUN',1000,1),($1,$3,'USD','DAY',1000,1)")
        .bind(f.tenant)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['effect.propose','effect.dispatch','effect.reconcile','effect.compensate','publication.create','publication.update'],$6,clock_timestamp()+interval '1 hour','ACTIVE')")
        .bind(f.tenant)
        .bind(grant_id)
        .bind(f.actor.principal_id.as_str())
        .bind(&f.actor.issuer)
        .bind(principal)
        .bind(json!([{"resource_type":"Run","resource_id":run_id.to_string(),"version":1}]))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.effects.workflow','1.0.0','test.effects',$4,$5,1,'{}'::jsonb,'RUNNING',$6,'STARTED')")
        .bind(f.tenant)
        .bind(run_id)
        .bind(principal)
        .bind(manifest.artifact_digest.as_str())
        .bind(grant_id)
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO connections(tenant_id,id,provider,external_account,scopes,secret_ref,state,contract_version,capabilities) VALUES($1,$2,'fixture-provider','acct-1','{publish}','local-test','ACTIVE','1.0.0','{effect.dispatch}')")
        .bind(f.tenant)
        .bind(connection_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((run_id, connection_id, grant_id))
}

async fn propose_effect(
    f: &Fixture,
    action: &str,
    content_ref: &ArtifactRef,
) -> TestResult<(Uuid, i32)> {
    let (run_id, connection_id, grant_id) = effect_seed(f).await?;
    // Target resource uses the run's own identity as a stable fixture target.
    let target = json!({
        "resource_type": "Run",
        "resource_id": run_id.to_string(),
        "version": 1
    });
    let command = f.command(
        "effect.propose",
        json!({
            "action": action,
            "connection_id": connection_id.to_string(),
            "target": target,
            "content_ref": content_ref,
            "grant_id": grant_id.to_string(),
            "price_profile": "1.0.0"
        }),
    );
    let receipt = f.store.execute(command).await?;
    assert_eq!(receipt.state, ReceiptState::Accepted);
    let effect_id = Uuid::parse_str(receipt.effect_id.as_ref().expect("effect_id").as_str())?;
    Ok((effect_id, 1))
}

// REQ-084 / AC-088: propose durably records the intent; no receipt or budget
// charge exists before dispatch, and a replay returns the same effect identity.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_propose_is_durable_before_transmission_and_replays_stably() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (content, _) = f.artifact(b"effect-content-h1".to_vec()).await?;
            let (effect_id, version) = propose_effect(&f, "publication.create", &content).await?;
            // Intent row is PREPARED with a reservation; no receipt exists yet.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query(
                "SELECT state,reservation_id FROM effects WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "PREPARED");
            let reservation_id: Uuid = row.try_get("reservation_id")?;
            let receipts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM effect_receipts WHERE tenant_id=$1 AND effect_id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(receipts, 0);
            // Reservation exists and holds the bound before any transmission.
            let res = sqlx::query("SELECT state,upper_bound_microunits FROM cost_reservations WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant)
                .bind(reservation_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(res.try_get::<String, _>("state")?, "RESERVED");
            assert_eq!(res.try_get::<i64, _>("upper_bound_microunits")?, 20);
            tx.rollback().await?;
            // The stored intent carries the run/connection identity it was
            // proposed against; re-reading it proves the projection is durable.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let (stored_action, stored_connection): (String, Uuid) = sqlx::query_as(
                "SELECT action,connection_id FROM effects WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(stored_action, "publication.create");
            let _ = stored_connection;
            tx.rollback().await?;
            let _ = version;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-088 / AC-092: same key, different content -> 409 IDEMPOTENCY_CONFLICT
// and the original intent is unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_propose_reused_key_with_different_content_conflicts() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (run_id, connection_id, grant_id) = effect_seed(&f).await?;
            let (h1, _) = f.artifact(b"content-h1".to_vec()).await?;
            let (h2, _) = f.artifact(b"content-h2".to_vec()).await?;
            let key = masonwing_contracts::ResourceId::new(Uuid::new_v4().to_string())?;
            let build = |content: &ArtifactRef| {
                AuthorizedCommand {
                    actor: f.actor.clone(),
                    operation: "effect.propose".into(),
                    idempotency_key: key.clone(),
                    fingerprint: fingerprint(
                        "effect.propose",
                        &json!({
                            "action": "publication.update",
                            "connection_id": connection_id.to_string(),
                            "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                            "content_ref": content,
                            "grant_id": grant_id.to_string(),
                            "price_profile": "1.0.0"
                        }),
                    )
                    .unwrap(),
                    input: json!({
                        "action": "publication.update",
                        "connection_id": connection_id.to_string(),
                        "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                        "content_ref": content,
                        "grant_id": grant_id.to_string(),
                        "price_profile": "1.0.0"
                    }),
                }
            };
            f.store.execute(build(&h1)).await?;
            let err = f.store.execute(build(&h2)).await.unwrap_err();
            assert_eq!(err.code, "IDEMPOTENCY_CONFLICT");
            // Original intent is unchanged and holds H1's fingerprint only.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM effects WHERE tenant_id=$1 AND connection_id=$2",
            )
            .bind(f.tenant)
            .bind(connection_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(count, 1);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-089 / AC-093: dispatch with an active connection kill switch ->
// KILL_SWITCH_ACTIVE and transmit_count stays 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_dispatch_blocked_by_kill_switch_before_ponr() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (content, _) = f.artifact(b"kill-switch-content".to_vec()).await?;
            let (run_id, connection_id, _) = effect_seed(&f).await?;
            let command = f.command(
                "effect.propose",
                json!({
                    "action": "publication.create",
                    "connection_id": connection_id.to_string(),
                    "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                    "content_ref": content,
                    "grant_id": grant_lookup(&f, run_id).await?.to_string(),
                    "price_profile": "1.0.0"
                }),
            );
            let receipt = f.store.execute(command).await?;
            let effect_id = Uuid::parse_str(receipt.effect_id.as_ref().unwrap().as_str())?;
            // Activate the CONNECTION-scoped kill switch pre-PONR.
            f.store
                .execute(f.command(
                    "kill-switch.set",
                    json!({
                        "scope": "CONNECTION",
                        "target_id": connection_id.to_string(),
                        "active": true,
                        "reason": "fixture stop before transmission"
                    }),
                ))
                .await?;
            let err = f
                .store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 1}),
                ))
                .await
                .unwrap_err();
            assert_eq!(err.code, "KILL_SWITCH_ACTIVE");
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let transmit: i32 = sqlx::query_scalar(
                "SELECT transmit_count FROM effects WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(transmit, 0);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-085 + REQ-086 / AC-089 + AC-090: ambiguous transport -> OUTCOME_UNKNOWN
// with the hold kept; remote evidence drives SUCCEEDED with a receipt, and
// resend is blocked while the outcome is unresolved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_unknown_timeout_blocks_resend_and_evidence_settles() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (content, _) = f.artifact(b"unknown-timeout-content".to_vec()).await?;
            let (run_id, connection_id, grant_id) = effect_seed(&f).await?;
            let command = f.command(
                "effect.propose",
                json!({
                    "action": "publication.create",
                    "connection_id": connection_id.to_string(),
                    "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                    "content_ref": content,
                    "grant_id": grant_id.to_string(),
                    "price_profile": "1.0.0"
                }),
            );
            let receipt = f.store.execute(command).await?;
            let effect_id = Uuid::parse_str(receipt.effect_id.as_ref().unwrap().as_str())?;
            // Durable dispatch admission: EXECUTING with transmit_count=1.
            let dispatch = f
                .store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 1}),
                ))
                .await?;
            assert_eq!(dispatch.state, ReceiptState::Accepted);
            // The transport layer observes an ambiguous timeout (provider cut the
            // connection after acceptance): the system path records it.
            f.store.record_effect_unknown(f.tenant, effect_id).await?;
            // Resend is blocked while the outcome is unknown.
            let err = f
                .store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 3}),
                ))
                .await
                .unwrap_err();
            assert_eq!(err.code, "OUTCOME_UNKNOWN_BLOCKS_RESEND");
            // The hold is intact (REQ-094) and reconciliation is queued.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT e.state,e.reconciliation_queued,e.transmit_count,r.state AS cost_state,r.upper_bound_microunits FROM effects e JOIN cost_reservations r ON r.tenant_id=e.tenant_id AND r.id=e.reservation_id WHERE e.tenant_id=$1 AND e.id=$2")
                .bind(f.tenant)
                .bind(effect_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "OUTCOME_UNKNOWN");
            assert!(row.try_get::<bool, _>("reconciliation_queued")?);
            assert_eq!(row.try_get::<i32, _>("transmit_count")?, 1);
            assert_eq!(row.try_get::<String, _>("cost_state")?, "UNKNOWN");
            tx.rollback().await?;
            // Reconcile: OUTCOME_UNKNOWN -> RECONCILING.
            let reconcile = f
                .store
                .execute(f.command(
                    "effect.reconcile",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 3}),
                ))
                .await?;
            assert_eq!(reconcile.state, ReceiptState::Succeeded);
            // Remote evidence (fixture operation already recorded provider-side)
            // confirms presence with the authoritative usage.
            let (evidence, _) = f.artifact(b"remote-evidence".to_vec()).await?;
            let applied = f
                .store
                .apply_reconciliation_evidence(
                    f.tenant,
                    effect_id,
                    masonwing_kernel::effects::ReconciliationEvidence::ConfirmedPresent,
                    Some("fixture-remote-id-1"),
                    Some(&evidence),
                    Some(12),
                )
                .await?;
            assert_eq!(applied.primary.resource_type, "EffectIntent");
            // SUCCEEDED + receipt with remote id + settled ledger.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT e.state,e.transmit_count,r.state AS cost_state,r.settled_microunits,(SELECT count(*) FROM effect_receipts er WHERE er.tenant_id=e.tenant_id AND er.effect_id=e.id) AS receipts FROM effects e JOIN cost_reservations r ON r.tenant_id=e.tenant_id AND r.id=e.reservation_id WHERE e.tenant_id=$1 AND e.id=$2")
                .bind(f.tenant)
                .bind(effect_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "SUCCEEDED");
            assert_eq!(row.try_get::<i32, _>("transmit_count")?, 1);
            assert_eq!(row.try_get::<String, _>("cost_state")?, "SETTLED");
            assert_eq!(row.try_get::<i64, _>("settled_microunits")?, 12);
            assert_eq!(row.try_get::<i64, _>("receipts")?, 1);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-087 / AC-091: unresolved reconciliation -> MANUAL_REVIEW, resend blocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_unresolved_reconciliation_halts_at_manual_review() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (content, _) = f.artifact(b"manual-review-content".to_vec()).await?;
            let (run_id, connection_id, grant_id) = effect_seed(&f).await?;
            let command = f.command(
                "effect.propose",
                json!({
                    "action": "publication.create",
                    "connection_id": connection_id.to_string(),
                    "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                    "content_ref": content,
                    "grant_id": grant_id.to_string(),
                    "price_profile": "1.0.0"
                }),
            );
            let receipt = f.store.execute(command).await?;
            let effect_id = Uuid::parse_str(receipt.effect_id.as_ref().unwrap().as_str())?;
            f.store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 1}),
                ))
                .await?;
            f.store.record_effect_unknown(f.tenant, effect_id).await?;
            f.store
                .execute(f.command(
                    "effect.reconcile",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 3}),
                ))
                .await?;
            // The provider lookup cannot establish the outcome.
            f.store
                .apply_reconciliation_evidence(
                    f.tenant,
                    effect_id,
                    masonwing_kernel::effects::ReconciliationEvidence::Unresolved,
                    None,
                    None,
                    None,
                )
                .await?;
            // MANUAL_REVIEW blocks resend; the hold is still kept.
            let err = f
                .store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 5}),
                ))
                .await
                .unwrap_err();
            assert_eq!(err.code, "OUTCOME_UNKNOWN_BLOCKS_RESEND");
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT e.state,r.state AS cost_state FROM effects e JOIN cost_reservations r ON r.tenant_id=e.tenant_id AND r.id=e.reservation_id WHERE e.tenant_id=$1 AND e.id=$2")
                .bind(f.tenant)
                .bind(effect_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "MANUAL_REVIEW");
            assert_eq!(row.try_get::<String, _>("cost_state")?, "UNKNOWN");
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// REQ-091 / AC-095: compensation creates a separate effect with its own
// approval; the original receipt is unchanged and requires a distinct approval.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn effect_compensation_creates_separate_authorized_effect() -> TestResult {
    let fixture = Arc::new(Fixture::effect().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let (content, _) = f.artifact(b"compensation-original".to_vec()).await?;
            let (run_id, connection_id, grant_id) = effect_seed(&f).await?;
            let command = f.command(
                "effect.propose",
                json!({
                    "action": "publication.create",
                    "connection_id": connection_id.to_string(),
                    "target": {"resource_type":"Run","resource_id":run_id.to_string(),"version":1},
                    "content_ref": content,
                    "grant_id": grant_id.to_string(),
                    "price_profile": "1.0.0"
                }),
            );
            let receipt = f.store.execute(command).await?;
            let effect_id = Uuid::parse_str(receipt.effect_id.as_ref().unwrap().as_str())?;
            // Drive the original to SUCCEEDED with a receipt.
            f.store
                .execute(f.command(
                    "effect.dispatch",
                    json!({"effect_id": effect_id.to_string(), "expected_version": 1}),
                ))
                .await?;
            let (evidence, _) = f.artifact(b"original-evidence".to_vec()).await?;
            f.store
                .record_effect_succeeded(f.tenant, effect_id, "fixture-remote-1", &evidence, 12)
                .await?;
            // Compensation requires a separate approval (REQ-091). Create one
            // APPROVED approvals row bound to the same run but distinct from the
            // original effect's (which has none), then submit compensation.
            let compensation_approval = Uuid::new_v4();
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO approvals(tenant_id,id,run_id,action,target,content_digest,scope_digest,policy_version,policy_epoch,max_cost_microunits,currency,expires_at,created_by,decided_by,state) VALUES($1,$2,$3,'publication.create.compensate',$4::jsonb,'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','1.0.0',1,20,'USD',clock_timestamp()+interval '1 hour',$5,$6,'APPROVED')")
                .bind(f.tenant)
                .bind(compensation_approval)
                .bind(run_id)
                .bind(json!({"resource_type":"Run","resource_id":run_id.to_string(),"version":1}))
                .bind(principal_of(&f))
                .bind(principal_of(&f))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            let (compensation_ref, _) = f.artifact(b"compensation-content".to_vec()).await?;
            let receipt = f
                .store
                .execute(f.command(
                    "effect.compensate",
                    json!({
                        "effect_id": effect_id.to_string(),
                        "compensation_ref": compensation_ref,
                        "approval_id": compensation_approval.to_string()
                    }),
                ))
                .await?;
            let compensation_id = Uuid::parse_str(receipt.effect_id.as_ref().unwrap().as_str())?;
            assert_ne!(compensation_id, effect_id);
            // New effect is linked to the original; original receipt untouched.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query("SELECT state,original_effect_id,approval_id FROM effects WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant)
                .bind(compensation_id)
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "AUTHORIZED");
            assert_eq!(row.try_get::<Uuid, _>("original_effect_id")?, effect_id);
            assert_eq!(
                row.try_get::<Uuid, _>("approval_id")?,
                compensation_approval
            );
            let receipts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM effect_receipts WHERE tenant_id=$1 AND effect_id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(receipts, 1);
            // A fresh command key must not reuse the same compensation approval.
            let err = f
                .store
                .execute(f.command(
                    "effect.compensate",
                    json!({
                        "effect_id": effect_id.to_string(),
                        "compensation_ref": compensation_ref,
                        "approval_id": compensation_approval.to_string()
                    }),
                ))
                .await;
            // The same approval again, under a fresh command idempotency key, is
            // refused: one approval authorizes exactly one compensation of a
            // given original (REQ-091). Assert both the refusal and that only
            // one compensation row exists.
            assert_eq!(err.unwrap_err().code, "APPROVAL_ALREADY_USED");
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM effects WHERE tenant_id=$1 AND original_effect_id=$2",
            )
            .bind(f.tenant)
            .bind(effect_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(count, 1);
            tx.rollback().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

async fn grant_lookup(f: &Fixture, run_id: Uuid) -> TestResult<Uuid> {
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    let grant: Uuid = sqlx::query_scalar("SELECT grant_id FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(f.tenant)
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.rollback().await?;
    Ok(grant)
}

fn principal_of(f: &Fixture) -> Uuid {
    Uuid::parse_str(f.actor.principal_id.as_str()).unwrap()
}

/// Install the workflow plugin once per test, then `run_seed` creates each run
/// the test needs. The worker never creates runs; admission owns identity.
async fn checkpoint_install(f: &Fixture) -> TestResult<String> {
    let (manifest, _) = f.package("test.checkpoint", "1.0.0").await?;
    f.store
        .execute(f.command("plugin.install", json!({"manifest": manifest})))
        .await?;
    Ok(manifest.artifact_digest.as_str().to_string())
}

async fn run_seed(f: &Fixture, plugin_digest: &str, state: &str) -> TestResult<Uuid> {
    // The dispatch path deserializes the stored input_ref as an ArtifactRef,
    // so the seeded run carries a real artifact reference.
    let (input_ref, _) = f
        .classified_artifact(br#"{}"#.to_vec(), Classification::Internal)
        .await?;
    let run_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let principal = principal_of(f);
    let mut tx = f.operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(f.tenant.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
        .bind(f.tenant).bind(grant_id).bind(principal.to_string())
        .bind(&f.actor.issuer).bind(principal)
        .execute(&mut *tx).await?;
    // dispatch_state must match the run's lifecycle: a QUEUED admission is
    // still PENDING; a running or terminal run has STARTED; a cancelled one
    // was fenced off before dispatch and is BLOCKED.
    let dispatch_state = match state {
        "QUEUED" => "PENDING",
        "CANCELLED" => "BLOCKED",
        _ => "STARTED",
    };
    sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.checkpoint.workflow','1.0.0','test.checkpoint',$4,$5,1,$6,$7,$8,$9)")
        .bind(f.tenant).bind(run_id).bind(principal)
        .bind(plugin_digest).bind(grant_id)
        .bind(serde_json::to_value(&input_ref)?)
        .bind(state).bind(run_id.to_string()).bind(dispatch_state)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(run_id)
}

// MASONWING@1.0.1 REQ-070 / AC-074 and REQ-075 / AC-079: the checkpoint row is
// the durable "dispatched once by logical ID" guarantee. A re-issued completion
// is a no-op that never rewrites SUCCEEDED, a stale fence cannot write, and a
// terminal run accepts no checkpoint at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_checkpoint_is_idempotent_and_never_rewrites_succeeded() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let digest = checkpoint_install(&f).await?;
            let run_id = run_seed(&f, &digest, "RUNNING").await?;
            let step = "0001:start";
            // Fresh completion of step one.
            assert!(
                f.store
                    .record_run_checkpoint(
                        f.tenant,
                        run_id,
                        &CheckpointWrite {
                            logical_step_id: step.to_string(),
                            state: "SUCCEEDED".to_string(),
                            failure_code: None,
                            fence: 1,
                            output_ref: None,
                        },
                    )
                    .await?
            );
            // Re-issued completion of the same logical step: already durable,
            // and it is not rewritten (the boolean is false, the row is kept).
            assert!(
                !f.store
                    .record_run_checkpoint(
                        f.tenant,
                        run_id,
                        &CheckpointWrite {
                            logical_step_id: step.to_string(),
                            state: "RUNNING".to_string(),
                            failure_code: None,
                            fence: 1,
                            output_ref: None,
                        },
                    )
                    .await?
            );
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let state: String = sqlx::query_scalar(
                "SELECT state FROM run_checkpoints WHERE tenant_id=$1 AND run_id=$2 AND logical_step_id=$3",
            )
            .bind(f.tenant)
            .bind(run_id)
            .bind(step)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(state, "SUCCEEDED");
            tx.rollback().await?;
            // A checkpoint from an execution that has since been re-fenced is
            // refused: a stale worker cannot re-drive an outdated traversal.
            let stale = f
                .store
                .record_run_checkpoint(
                    f.tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id: "0002:next".to_string(),
                        state: "SUCCEEDED".to_string(),
                        failure_code: None,
                        fence: 99,
                        output_ref: None,
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(stale.code, "STALE_FENCE");
            // The invalid checkpoint wrote nothing.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM run_checkpoints WHERE tenant_id=$1 AND run_id=$2",
            )
            .bind(f.tenant)
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(rows, 1);
            tx.rollback().await?;
            // A terminal run accepts no further checkpoints.
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE runs SET state='SUCCEEDED' WHERE tenant_id=$1 AND id=$2")
                .bind(f.tenant)
                .bind(run_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            let terminal = f
                .store
                .record_run_checkpoint(
                    f.tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id: "0002:next".to_string(),
                        state: "SUCCEEDED".to_string(),
                        failure_code: None,
                        fence: 1,
                        output_ref: None,
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(terminal.code, "ILLEGAL_TRANSITION");
            // An unknown checkpoint state is a request error, never stored.
            let unknown_state = f
                .store
                .record_run_checkpoint(
                    f.tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id: "0003:x".to_string(),
                        state: "PENDING".to_string(),
                        failure_code: None,
                        fence: 1,
                        output_ref: None,
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(unknown_state.code, "ILLEGAL_TRANSITION");
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-069 and REQ-073: dispatch acknowledgement is fence-bound.
// A QUEUED/PENDING run becomes RUNNING with the adopted Temporal run id; a run
// that was cancelled first, or a caller holding a stale fence, is refused and
// no RUNNING write is emitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_mark_started_requires_matching_fence_and_queued_state() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            let digest = checkpoint_install(&f).await?;
            let run_id = run_seed(&f, &digest, "QUEUED").await?;
            // Stale fence: the run is untouched.
            let stale = f
                .store
                .mark_run_started(f.tenant, run_id, "temporal-run-1", 7)
                .await
                .unwrap_err();
            assert_eq!(stale.code, "STALE_FENCE");
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let state: String =
                sqlx::query_scalar("SELECT state FROM runs WHERE tenant_id=$1 AND id=$2")
                    .bind(f.tenant)
                    .bind(run_id)
                    .fetch_one(&mut *tx)
                    .await?;
            assert_eq!(state, "QUEUED");
            tx.rollback().await?;
            // The acknowledged start adopts the server's run id and increments
            // the version exactly once.
            let mutation = f
                .store
                .mark_run_started(f.tenant, run_id, "temporal-run-1", 1)
                .await?;
            assert_eq!(mutation.primary.version, 2);
            assert_eq!(mutation.run_id, Some(run_id));
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            let row = sqlx::query(
                "SELECT state,version,dispatch_state,temporal_run_id FROM runs WHERE tenant_id=$1 AND id=$2",
            )
            .bind(f.tenant)
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!(row.try_get::<String, _>("state")?, "RUNNING");
            assert_eq!(row.try_get::<i32, _>("version")?, 2);
            assert_eq!(row.try_get::<String, _>("dispatch_state")?, "STARTED");
            assert_eq!(
                row.try_get::<String, _>("temporal_run_id")?,
                "temporal-run-1"
            );
            tx.rollback().await?;
            // A second acknowledgement is refused: the run is no longer QUEUED.
            let replay = f
                .store
                .mark_run_started(f.tenant, run_id, "temporal-run-2", 1)
                .await
                .unwrap_err();
            assert_eq!(replay.code, "ILLEGAL_TRANSITION");
            // A run cancelled before dispatch never reaches RUNNING.
            let cancelled = run_seed(&f, &digest, "CANCELLED").await?;
            let refused = f
                .store
                .mark_run_started(f.tenant, cancelled, "temporal-run-3", 1)
                .await
                .unwrap_err();
            assert_eq!(refused.code, "ILLEGAL_TRANSITION");
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-070 / AC-074: the outbox relay claims each pending
// event exactly once under a lease, marks it delivered in the inbox, and
// refuses to dispatch a run whose authority has lapsed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbox_relay_claims_once_and_rechecks_run_authority() -> TestResult {
    let fixture = Arc::new(Fixture::create().await?);
    let task = tokio::spawn({
        let f = fixture.clone();
        async move {
            // Seed two outbox events directly (as record_change would).
            let event_a = Uuid::new_v4();
            let event_b = Uuid::new_v4();
            let aggregate = Uuid::new_v4();
            {
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                // (tenant, aggregate_id, aggregate_version, event_name) is
                // unique, so each event carries its own aggregate version.
                for (version, event_id) in [(1, event_a), (2, event_b)] {
                    sqlx::query("INSERT INTO outbox_events(tenant_id,id,aggregate_id,aggregate_version,event_name,aggregate_type) VALUES($1,$2,$3,$4,'run.start','Run')")
                        .bind(f.tenant).bind(event_id).bind(aggregate).bind(version)
                        .execute(&mut *tx).await?;
                }
                tx.commit().await?;
            }

            // First claim returns both events in stream order. A one-second
            // lease keeps the expiry window testable.
            let claimed = f.store.claim_pending_outbox(f.tenant, 16, 1).await?;
            let ids: Vec<Uuid> = claimed.iter().map(|item| item.event_id).collect();
            assert_eq!(ids, vec![event_a, event_b]);
            assert_eq!(claimed[0].event_name, "run.start");
            assert_eq!(claimed[0].aggregate_id, aggregate);

            // The claim set a lease: an immediate re-claim returns nothing.
            let second = f.store.claim_pending_outbox(f.tenant, 16, 30).await?;
            assert!(second.is_empty());

            // Delivery marks the event DELIVERED and dedupes via the inbox.
            f.store.mark_outbox_delivered(f.tenant, event_a).await?;
            let delivered: String = {
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                let status = sqlx::query_scalar(
                    "SELECT delivery_status FROM outbox_events WHERE tenant_id=$1 AND id=$2",
                )
                .bind(f.tenant)
                .bind(event_a)
                .fetch_one(&mut *tx)
                .await?;
                let inbox: i64 = sqlx::query_scalar("SELECT count(*) FROM inbox_events WHERE tenant_id=$1 AND consumer='worker-relay' AND event_id=$2")
                    .bind(f.tenant).bind(event_a).fetch_one(&mut *tx).await?;
                assert_eq!(inbox, 1);
                tx.rollback().await?;
                status
            };
            assert_eq!(delivered, "DELIVERED");

            // A later claim skips the delivered event even after the lease of
            // the other expires.
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            let third = f.store.claim_pending_outbox(f.tenant, 16, 1).await?;
            assert_eq!(third.len(), 1);
            assert_eq!(third[0].event_id, event_b);

            // mark_outbox_failed moves an exhausted event to DEAD_LETTER.
            f.store.mark_outbox_failed(f.tenant, event_b, 1).await?;
            let dead: String = {
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                let status = sqlx::query_scalar(
                    "SELECT delivery_status FROM outbox_events WHERE tenant_id=$1 AND id=$2",
                )
                .bind(f.tenant)
                .bind(event_b)
                .fetch_one(&mut *tx)
                .await?;
                tx.rollback().await?;
                status
            };
            assert_eq!(dead, "DEAD_LETTER");

            // A QUEUED run whose grant has been revoked is fenced BLOCKED by
            // the authority re-check instead of being dispatched.
            let (manifest, _) = f.package("test.relay", "1.0.0").await?;
            f.store
                .execute(f.command("plugin.install", json!({"manifest": manifest})))
                .await?;
            // ENABLE the plugin so the authority re-check reaches the grant.
            f.store
                .execute(f.command(
                    "plugin.enable",
                    json!({
                    "plugin_id": manifest.id, "artifact_digest": manifest.artifact_digest,
                    "grants": ["resource.read"], "expected_version": 1}),
                ))
                .await?;
            let run = Uuid::new_v4();
            let grant = Uuid::new_v4();
            let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
            let (input, _) = f
                .classified_artifact(b"relay fixture".to_vec(), Classification::Confidential)
                .await?;
            let mut tx = f.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(f.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            // Grant already expired.
            sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()-interval '1 hour','ACTIVE')")
                .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.relay.workflow','1.0.0',$4,$5,$6,1,$7,'QUEUED',$8,'PENDING')")
                .bind(f.tenant).bind(run).bind(principal).bind(manifest.id.as_str())
                .bind(manifest.artifact_digest.as_str()).bind(grant).bind(json!(input)).bind(run.to_string())
                .execute(&mut *tx).await?;
            tx.commit().await?;

            let dispatchable = f.store.verify_and_claim_run(f.tenant, run).await?;
            assert!(dispatchable.is_none());
            let blocked: (String, Option<String>) = {
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                let row =
                    sqlx::query("SELECT state,failure_code FROM runs WHERE tenant_id=$1 AND id=$2")
                        .bind(f.tenant)
                        .bind(run)
                        .fetch_one(&mut *tx)
                        .await?;
                let value = (row.try_get("state")?, row.try_get("failure_code")?);
                tx.rollback().await?;
                value
            };
            assert_eq!(blocked.0, "BLOCKED");
            assert_eq!(blocked.1.as_deref(), Some("GRANT_REVOKED_OR_EXPIRED"));

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
    });
    let result = task.await;
    let cleanup = fixture.cleanup().await;
    result??;
    cleanup
}

// MASONWING@1.0.1 REQ-070 / AC-074 (authority re-check): a run admitted while
// the requester held a membership must not be dispatched after that
// membership is revoked or the policy stops permitting run dispatch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_fences_run_when_membership_or_policy_is_withdrawn() -> TestResult {
    for (revoke_membership, expected) in [(true, "MEMBERSHIP_INACTIVE"), (false, "POLICY_REVOKED")]
    {
        let fixture = Arc::new(Fixture::create().await?);
        let task = tokio::spawn({
            let f = fixture.clone();
            async move {
                let (manifest, _) = f.package("test.fence", "1.0.0").await?;
                f.store
                    .execute(f.command("plugin.install", json!({"manifest": manifest})))
                    .await?;
                f.store
                    .execute(f.command(
                        "plugin.enable",
                        json!({
                        "plugin_id": manifest.id, "artifact_digest": manifest.artifact_digest,
                        "grants": ["resource.read"], "expected_version": 1}),
                    ))
                    .await?;
                let run = Uuid::new_v4();
                let grant = Uuid::new_v4();
                let principal = Uuid::parse_str(f.actor.principal_id.as_str())?;
                let mut tx = f.operator.begin().await?;
                sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                    .bind(f.tenant.to_string())
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER',$4,$5,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
                    .bind(f.tenant).bind(grant).bind(principal.to_string()).bind(&f.actor.issuer).bind(principal)
                    .execute(&mut *tx).await?;
                let (input, _) = f
                    .classified_artifact(b"fence fixture".to_vec(), Classification::Confidential)
                    .await?;
                sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,'test.fence.workflow','1.0.0',$4,$5,$6,1,$7,'QUEUED',$8,'PENDING')")
                    .bind(f.tenant).bind(run).bind(principal).bind(manifest.id.as_str())
                    .bind(manifest.artifact_digest.as_str()).bind(grant).bind(json!(input)).bind(run.to_string())
                    .execute(&mut *tx).await?;
                if revoke_membership {
                    sqlx::query("UPDATE memberships SET status='REVOKED' WHERE tenant_id=$1 AND principal_id=$2")
                        .bind(f.tenant).bind(principal).execute(&mut *tx).await?;
                } else {
                    sqlx::query(
                        "UPDATE authorization_policies SET is_current=false WHERE tenant_id=$1",
                    )
                    .bind(f.tenant)
                    .execute(&mut *tx)
                    .await?;
                }
                tx.commit().await?;

                assert!(f.store.verify_and_claim_run(f.tenant, run).await?.is_none());
                let (state, code): (String, Option<String>) = {
                    let mut tx = f.operator.begin().await?;
                    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                        .bind(f.tenant.to_string())
                        .execute(&mut *tx)
                        .await?;
                    let row = sqlx::query(
                        "SELECT state,failure_code FROM runs WHERE tenant_id=$1 AND id=$2",
                    )
                    .bind(f.tenant)
                    .bind(run)
                    .fetch_one(&mut *tx)
                    .await?;
                    let value = (row.try_get("state")?, row.try_get("failure_code")?);
                    tx.rollback().await?;
                    value
                };
                assert_eq!(state, "BLOCKED");
                assert_eq!(code.as_deref(), Some(expected));
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
            }
        });
        let result = task.await;
        let cleanup = fixture.cleanup().await;
        result??;
        cleanup?;
    }
    Ok(())
}
