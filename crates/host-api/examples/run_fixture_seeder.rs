//! Operator-side fixture seeder for the TC-AC-072 / TC-AC-073 acceptance drivers.
//!
//! The acceptance driver owns tenant creation over HTTP-adjacent operator SQL
//! (tenant + OWNER membership for the real signed-in OIDC principal). This
//! example then installs the checksum plugin, seeds the run input artifact and
//! creates a grant delegated to that same principal, so the subsequent
//! `POST /commands/run.start` over the real BFF admits against real rows.
//!
//! Two modes, both printing one JSON object on stdout:
//!
//!   --seed --tenant-id <uuid> --principal-id <uuid>
//!       Seed plugin + grant + input artifact into an existing tenant.
//!       Prints the run.start payload the driver posts.
//!
//!   --cleanup --tenant-id <uuid>
//!       Remove every seeded row for that tenant and the tenant itself.
//!
//! No secret, OAuth code, cookie or CSRF value is printed or persisted.

use std::{env, sync::Arc, time::Duration};

use ed25519_dalek::{Signer, SigningKey};
use masonwing_artifacts_adapter::{ArtifactStoreAdapter, S3Config};
use masonwing_contract_validation::{canonical_bytes, digest_bytes, fingerprint};
use masonwing_contracts::{
    ArtifactId, PrincipalId, ResourceId, TenantId,
    wire::{ArtifactRef, Classification, PluginManifest},
};
use masonwing_data_postgres::{
    PostgresStore,
    registry_validation::{manifest_digest, manifest_signing_message},
};
use masonwing_host_api::{
    fixture_import::materialize_fixture,
    plugin_runtime::{HostPluginRuntime, local_fixture_plugins},
};
use masonwing_kernel::runtime::{
    ArtifactObjects, AuthorizedCommand, CommandActor, CommandRepository,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Fixture {
    operator: PgPool,
    store: PostgresStore,
    objects: Arc<ArtifactStoreAdapter>,
    author: CommandActor,
    tenant: Uuid,
}

fn argument(name: &str) -> Result<String> {
    let mut args = env::args().skip(1);
    while let Some(value) = args.next() {
        if value == name {
            return args
                .next()
                .ok_or_else(|| format!("{name} requires a value").into());
        }
    }
    Err(format!("missing required argument {name}").into())
}

fn has_flag(name: &str) -> bool {
    env::args().skip(1).any(|value| value == name)
}

impl Fixture {
    async fn connect(tenant: Uuid, principal: Uuid) -> Result<Self> {
        let operator = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .connect(
                "postgresql://masonwing_migrator:masonwing-local-migrator@127.0.0.1:39852/masonwing",
            )
            .await?;
        let runtime = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect("postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing")
            .await?;
        let objects = Arc::new(ArtifactStoreAdapter::from_s3(S3Config::new(
            "http://127.0.0.1:39856",
            "masonwing-artifacts-local",
            "us-east-1",
            env::var("MINIO_ROOT_USER").unwrap_or_else(|_| "masonwing-local".into()),
            env::var("MINIO_ROOT_PASSWORD").unwrap_or_else(|_| "masonwing-local-storage".into()),
            true,
        )?)?);
        let plugin_runtime = HostPluginRuntime::new(true)?;
        let native = plugin_runtime.native_packages();
        let store = PostgresStore::from_pool(runtime, objects.clone())
            .await?
            .with_native_packages(native)
            .with_plugin_runtime(Arc::new(plugin_runtime));
        Ok(Self {
            operator,
            store,
            objects,
            author: CommandActor {
                tenant_id: TenantId::new(tenant.to_string())?,
                principal_id: PrincipalId::new(principal.to_string())?,
                issuer: "https://local-fixture.example.test".into(),
                membership_epoch: 1,
                permission_epoch: 1,
                policy_version: "1.0.0".into(),
                policy_epoch: 1,
            },
            tenant,
        })
    }

    fn command(&self, operation: &str, input: Value) -> AuthorizedCommand {
        AuthorizedCommand {
            actor: self.author.clone(),
            operation: operation.into(),
            idempotency_key: ResourceId::new(Uuid::new_v4().to_string()).unwrap(),
            fingerprint: fingerprint(operation, &input).unwrap(),
            input,
        }
    }

    async fn seed_artifact(&self, reference: &ArtifactRef, bytes: Vec<u8>) -> Result {
        if reference.digest != digest_bytes(&bytes) || reference.tenant_id != self.author.tenant_id
        {
            return Err("fixture artifact binding failed".into());
        }
        let id = Uuid::parse_str(reference.artifact_id.as_str())?;
        let key = format!(
            "{}/{}/{}",
            self.tenant,
            id,
            reference.digest.as_str().trim_start_matches("sha256:")
        );
        self.objects
            .put_immutable(&key, bytes.clone(), "application/json")
            .await?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifacts(tenant_id,id,digest,classification,content_type,size_bytes,storage_key,state,creator_id) VALUES($1,$2,$3,$4,'application/json',$5,$6,'ACTIVE',$7)")
            .bind(self.tenant).bind(id).bind(reference.digest.as_str()).bind(reference.classification.as_str()).bind(bytes.len() as i64).bind(key)
            .bind(Uuid::parse_str(self.author.principal_id.as_str())?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    fn reference(&self, bytes: &[u8], classification: Classification) -> ArtifactRef {
        ArtifactRef {
            artifact_id: ArtifactId::new(Uuid::new_v4().to_string()).unwrap(),
            tenant_id: self.author.tenant_id.clone(),
            digest: digest_bytes(bytes),
            schema_version: "1.0.0".into(),
            classification,
        }
    }

    /// Install the packaged checksum fixture and enable it, returning the
    /// workflow identity the run will pin plus the upgraded manifest for
    /// plugin.upgrade idempotency probes.
    async fn install_workflow_plugin(&self) -> Result<(String, String, PluginManifest)> {
        let signer = SigningKey::from_bytes(&[43; 32]);
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'masonwing.first-party','local.integration',$2) ON CONFLICT DO NOTHING")
            .bind(self.tenant).bind(signer.verifying_key().to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        let (fixture, _) = local_fixture_plugins(&self.author.tenant_id)
            .into_iter()
            .find(|(f, _)| f.plugin_id.as_str() == "fixture.checksum")
            .ok_or("checksum fixture missing")?;
        let fixture = materialize_fixture(fixture)?;
        for artifact in &fixture.supporting_artifacts {
            self.seed_artifact(&artifact.reference, artifact.bytes.clone())
                .await?;
        }
        let package_ref = self.reference(fixture.artifact_bytes, Classification::Internal);
        self.seed_artifact(&package_ref, fixture.artifact_bytes.to_vec())
            .await?;
        let sbom = canonical_bytes(
            &json!({"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"type":"library","name":fixture.plugin_id,"version":fixture.version}]}),
        )?;
        let sbom_ref = self.reference(&sbom, Classification::Internal);
        self.seed_artifact(&sbom_ref, sbom).await?;

        let manifest: PluginManifest = serde_json::from_value(
            json!({"id":fixture.plugin_id,"version":fixture.version,"contract_version":fixture.contract_version,"artifact_digest":fixture.artifact_digest,
            "publisher_id":fixture.publisher_id,"execution_class":fixture.execution_class,"requested_capabilities":[],"dependencies":[],"ui":[],"migrations":[],"license_expression":"Apache-2.0",
            "sbom_digest":sbom_ref.digest,"signature_ref":Uuid::new_v4().to_string(),"handlers":fixture.handlers,"workflows":fixture.workflows,"data_contracts":[],"tools":[]}),
        )?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_signatures(tenant_id,id,publisher_id,key_id,artifact_digest,manifest_digest,signature) VALUES($1,$2,'masonwing.first-party','local.integration',$3,$4,$5)")
            .bind(self.tenant).bind(&manifest.signature_ref).bind(manifest.artifact_digest.as_str()).bind(manifest_digest(&manifest)?.as_str())
            .bind(signer.sign(&manifest_signing_message(&manifest)?).to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        self.store
            .execute(self.command("plugin.install", json!({"manifest":manifest})))
            .await?;
        self.store
            .execute(self.command("plugin.enable", json!({"plugin_id":manifest.id,"artifact_digest":manifest.artifact_digest,"grants":[],"expected_version":1})))
            .await?;

        // Prepare upgraded manifest for plugin.upgrade idempotency probes
        let v2_package = b"fixture.checksum package v2".to_vec();
        let v2_package_ref = self.reference(&v2_package, Classification::Internal);
        self.seed_artifact(&v2_package_ref, v2_package).await?;

        let v2_sbom = canonical_bytes(
            &json!({"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"type":"library","name":fixture.plugin_id,"version":"2.0.0"}]}),
        )?;
        let v2_sbom_ref = self.reference(&v2_sbom, Classification::Internal);
        self.seed_artifact(&v2_sbom_ref, v2_sbom).await?;

        let manifest_v2: PluginManifest = serde_json::from_value(json!({
            "id": fixture.plugin_id,
            "version": "2.0.0",
            "contract_version": fixture.contract_version,
            "artifact_digest": v2_package_ref.digest,
            "publisher_id": fixture.publisher_id,
            "execution_class": "WASM_COMPONENT",
            "requested_capabilities": [],
            "dependencies": [],
            "ui": [],
            "migrations": [],
            "license_expression": "Apache-2.0",
            "sbom_digest": v2_sbom_ref.digest,
            "signature_ref": Uuid::new_v4().to_string(),
            "handlers": [],
            "workflows": [],
            "data_contracts": [],
            "tools": []
        }))?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_signatures(tenant_id,id,publisher_id,key_id,artifact_digest,manifest_digest,signature) VALUES($1,$2,'masonwing.first-party','local.integration',$3,$4,$5)")
            .bind(self.tenant).bind(&manifest_v2.signature_ref).bind(manifest_v2.artifact_digest.as_str()).bind(manifest_digest(&manifest_v2)?.as_str())
            .bind(signer.sign(&manifest_signing_message(&manifest_v2)?).to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        Ok((
            "fixture.checksum.workflow.v1".into(),
            "1.0.0".into(),
            manifest_v2,
        ))
    }

    /// Install the packaged document-review fixture and leave it in its
    /// post-install state INSTALLED_DISABLED. This gives acceptance drivers a
    /// real persisted from-state (installed but never enabled) without any
    /// direct state write: the row exists only because plugin.install ran.
    async fn install_disabled_plugin(&self) -> Result<String> {
        let signer = SigningKey::from_bytes(&[43; 32]);
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'masonwing.first-party','local.integration',$2) ON CONFLICT DO NOTHING")
            .bind(self.tenant).bind(signer.verifying_key().to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        let (fixture, _) = local_fixture_plugins(&self.author.tenant_id)
            .into_iter()
            .find(|(f, _)| f.plugin_id.as_str() == "fixture.document-review")
            .ok_or("document-review fixture missing")?;
        let fixture = materialize_fixture(fixture)?;
        for artifact in &fixture.supporting_artifacts {
            self.seed_artifact(&artifact.reference, artifact.bytes.clone())
                .await?;
        }
        let package_ref = self.reference(fixture.artifact_bytes, Classification::Internal);
        self.seed_artifact(&package_ref, fixture.artifact_bytes.to_vec())
            .await?;
        let sbom = canonical_bytes(
            &json!({"bomFormat":"CycloneDX","specVersion":"1.6","components":[{"type":"library","name":fixture.plugin_id,"version":fixture.version}]}),
        )?;
        let sbom_ref = self.reference(&sbom, Classification::Internal);
        self.seed_artifact(&sbom_ref, sbom).await?;

        let manifest: PluginManifest = serde_json::from_value(
            json!({"id":fixture.plugin_id,"version":fixture.version,"contract_version":fixture.contract_version,"artifact_digest":fixture.artifact_digest,
            "publisher_id":fixture.publisher_id,"execution_class":fixture.execution_class,"requested_capabilities":[],"dependencies":[],"ui":[],"migrations":[],"license_expression":"Apache-2.0",
            "sbom_digest":sbom_ref.digest,"signature_ref":Uuid::new_v4().to_string(),"handlers":fixture.handlers,"workflows":fixture.workflows,"data_contracts":[],"tools":[]}),
        )?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_signatures(tenant_id,id,publisher_id,key_id,artifact_digest,manifest_digest,signature) VALUES($1,$2,'masonwing.first-party','local.integration',$3,$4,$5)")
            .bind(self.tenant).bind(&manifest.signature_ref).bind(manifest.artifact_digest.as_str()).bind(manifest_digest(&manifest)?.as_str())
            .bind(signer.sign(&manifest_signing_message(&manifest)?).to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        // Install only: the row lands INSTALLED_DISABLED (version 1). No enable.
        self.store
            .execute(self.command("plugin.install", json!({"manifest":manifest})))
            .await?;
        Ok(fixture.plugin_id.to_string())
    }

    async fn seed(&self) -> Result<Value> {
        let disabled_plugin_id = self.install_disabled_plugin().await?;
        let (workflow_id, workflow_version, upgrade_manifest) =
            self.install_workflow_plugin().await?;

        // The checksum workflow's pinned input schema requires a JSON array of
        // integers in 0..=255. The definition is digest-pinned; supply input
        // that satisfies it as-is rather than mutating the definition.
        let input_bytes = canonical_bytes(&json!([82, 117, 110]))?;
        let input_ref = self.reference(&input_bytes, Classification::Confidential);
        self.seed_artifact(&input_ref, input_bytes).await?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_rights(tenant_id,artifact_id,rights_id,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,state) VALUES($1,$2,$3,'FIRST_PARTY',true,false,true,false,'ACTIVE')")
            .bind(self.tenant).bind(Uuid::parse_str(input_ref.artifact_id.as_str())?).bind(Uuid::new_v4()).execute(&mut *tx).await?;
        tx.commit().await?;

        let target_bytes = canonical_bytes(&json!({"doc":"draft v1"}))?;
        let target_ref = self.reference(&target_bytes, Classification::Confidential);
        self.seed_artifact(&target_ref, target_bytes).await?;

        let grant = self
            .store
            .execute(self.command("grant.create", json!({
                "delegate":{"type":"USER","id":self.author.principal_id,"issuer":self.author.issuer},
                "actions":["run.start","effect.propose","effect.dispatch","artifact.read","artifact.write"],
                "resources":[
                    {"resource_type":"Artifact","resource_id":input_ref.artifact_id.as_str(),"version":1},
                    {"resource_type":"Artifact","resource_id":target_ref.artifact_id.as_str(),"version":1}
                ],
                "parent_grant_id":null,
                "expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)})))
            .await?
            .resource
            .ok_or("missing grant receipt")?;

        Ok(json!({
            "tenant_id": self.tenant.to_string(),
            "grant_id": grant.resource_id.as_str(),
            "workflow_id": workflow_id,
            "workflow_version": workflow_version,
            "input_ref": input_ref,
            "disabled_plugin_id": disabled_plugin_id,
            "upgrade_manifest": upgrade_manifest,
            "run_start_payload": {
                "workflow_id": workflow_id,
                "workflow_version": workflow_version,
                "input_ref": input_ref,
                "grant_id": grant.resource_id.as_str(),
            },
        }))
    }

    async fn cleanup(&self) -> Result {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        for table in [
            "effect_receipts",
            "effects",
            "approvals",
            "cost_ledger",
            "cost_reservations",
            "budget_accounts",
            "budget_settings",
            "connector_authorizations",
            "ui_contributions",
            "provider_profiles",
            "run_checkpoints",
            "runs",
            "connections",
            "deletion_requests",
            "deletion_tombstones",
            "kill_switches",
            "notifications",
            "membership_invites",
            "uploads",
            "resource_projections",
            "artifact_lineage",
            "artifact_rights",
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
            "artifacts",
            "membership_roles",
            "memberships",
            "authorization_policies",
            "catalog_listings",
            "identity_configuration_proposals",
            "policy_proposals",
            "policy_configuration_state",
            "support_grants",
            "inbox_events",
            "read_cursors",
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
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result {
    let tenant: Uuid = argument("--tenant-id")?.parse()?;
    let cleanup = has_flag("--cleanup");
    let relay = has_flag("--relay");
    let principal: Uuid = if cleanup || relay {
        Uuid::nil()
    } else {
        argument("--principal-id")?.parse()?
    };
    let fixture = Fixture::connect(tenant, principal).await?;
    if cleanup {
        fixture.cleanup().await?;
        println!("{}", json!({"cleaned": tenant.to_string()}));
        return Ok(());
    }
    if relay {
        let config = masonwing_worker::RelayConfig {
            database_url:
                "postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing".into(),
            tenants: vec![tenant],
            temporal_address: "http://127.0.0.1:39854".into(),
            temporal_namespace: "masonwing-local".into(),
            task_queue: "masonwing-runs".into(),
            poll_interval: Duration::from_millis(50),
            batch_size: 16,
            lease_seconds: 30,
            max_attempts: 10,
        };
        let dispatched = masonwing_worker::relay_once(&fixture.store, &config).await;
        println!("{}", json!({"dispatched": dispatched}));
        return Ok(());
    }
    let output = fixture.seed().await?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
