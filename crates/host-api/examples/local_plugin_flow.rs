//! Reproducible local application integration: real PostgreSQL, real MinIO,
//! registry signature validation and both actual native SDK fixture handlers.
//! It creates/deletes only its own random tenant. No HTTP/OIDC or Temporal claim.
//! Run from repository root: cargo run -p masonwing-host-api --example local_plugin_flow -- --local

use std::{sync::Arc, time::Duration};

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

struct LocalFlow {
    operator: PgPool,
    store: PostgresStore,
    objects: Arc<ArtifactStoreAdapter>,
    actor: CommandActor,
    tenant: Uuid,
}

#[tokio::main]
async fn main() -> Result {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--local"] {
        return Err("usage: local_plugin_flow --local (fixed local endpoints only)".into());
    }
    let flow = Arc::new(LocalFlow::create().await?);
    let work = tokio::spawn({
        let flow = flow.clone();
        async move { flow.exercise().await }
    })
    .await;
    let cleanup = flow.cleanup().await;
    let report = work??;
    cleanup?;
    std::fs::create_dir_all(".evidence/plugin-runtime")?;
    let output = serde_json::to_string_pretty(&report)?;
    std::fs::write(".evidence/plugin-runtime/local-flow.json", &output)?;
    println!("{output}");
    Ok(())
}

impl LocalFlow {
    async fn create() -> Result<Self> {
        let operator=PgPoolOptions::new().max_connections(2).acquire_timeout(Duration::from_secs(5))
            .connect("postgresql://masonwing_migrator:masonwing-local-migrator@127.0.0.1:39852/masonwing").await?;
        let runtime = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect("postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing")
            .await?;
        let objects = Arc::new(ArtifactStoreAdapter::from_s3(S3Config::new(
            "http://127.0.0.1:39856",
            "masonwing-artifacts-local",
            "us-east-1",
            std::env::var("MINIO_ROOT_USER").unwrap_or_else(|_| "masonwing-local".into()),
            std::env::var("MINIO_ROOT_PASSWORD")
                .unwrap_or_else(|_| "masonwing-local-storage".into()),
            true,
        )?)?);
        let plugin_runtime = HostPluginRuntime::new(true)?;
        let native = plugin_runtime.native_packages();
        let store = PostgresStore::from_pool(runtime, objects.clone())
            .await?
            .with_native_packages(native)
            .with_plugin_runtime(Arc::new(plugin_runtime));
        let tenant = Uuid::new_v4();
        let principal = Uuid::new_v4();
        let membership = Uuid::new_v4();
        let actor = CommandActor {
            tenant_id: TenantId::new(tenant.to_string())?,
            principal_id: PrincipalId::new(principal.to_string())?,
            issuer: "https://local-fixture.example.test".into(),
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
        sqlx::query(
            "INSERT INTO tenants(id,name) VALUES($1,'ephemeral native integration fixture')",
        )
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
        sqlx::query("INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current) VALUES($1,'1.0.0',1,'permit(principal, action, resource);',true)")
            .bind(tenant).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self {
            operator,
            store,
            objects,
            actor,
            tenant,
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

    async fn seed_artifact(&self, reference: &ArtifactRef, bytes: Vec<u8>) -> Result {
        if reference.digest != digest_bytes(&bytes) || reference.tenant_id != self.actor.tenant_id {
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
            .bind(Uuid::parse_str(self.actor.principal_id.as_str())?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    fn reference(&self, bytes: &[u8], classification: Classification) -> ArtifactRef {
        ArtifactRef {
            artifact_id: ArtifactId::new(Uuid::new_v4().to_string()).unwrap(),
            tenant_id: self.actor.tenant_id.clone(),
            digest: digest_bytes(bytes),
            schema_version: "1.0.0".into(),
            classification,
        }
    }

    async fn exercise(&self) -> Result<Value> {
        // This public synthetic seed is confined to a new ephemeral local tenant;
        // it is not a production publisher key or an HTTP registration endpoint.
        let signer = SigningKey::from_bytes(&[43; 32]);
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'masonwing.first-party','local.integration',$2)")
            .bind(self.tenant).bind(signer.verifying_key().to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;
        let input_bytes = canonical_bytes(&b"Masonwing local native proof".to_vec())?;
        let input = self.reference(&input_bytes, Classification::Confidential);
        self.seed_artifact(&input, input_bytes).await?;
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_rights(tenant_id,artifact_id,rights_id,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,state) VALUES($1,$2,$3,'FIRST_PARTY',true,false,true,false,'ACTIVE')")
            .bind(self.tenant).bind(Uuid::parse_str(input.artifact_id.as_str())?).bind(Uuid::new_v4()).execute(&mut *tx).await?;
        tx.commit().await?;
        let mut results = Vec::new();
        let mut locked = Vec::new();
        for (fixture, _) in local_fixture_plugins(&self.actor.tenant_id) {
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
            let installed=self.store.execute(self.command("plugin.enable",json!({"plugin_id":manifest.id,"artifact_digest":manifest.artifact_digest,"grants":[],"expected_version":1}))).await?.resource.ok_or("missing installation receipt")?;
            let grant=self.store.execute(self.command("grant.create",json!({"delegate":{"type":"USER","id":self.actor.principal_id,"issuer":self.actor.issuer},"actions":["plugin.invoke","artifact.read"],
                "resources":[installed,{"resource_type":"Artifact","resource_id":input.artifact_id,"version":1}],"parent_grant_id":null,
                "expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)}))).await?.resource.ok_or("missing grant receipt")?;
            let command=self.command("plugin.invoke",json!({"plugin_id":manifest.id,"handler":manifest.handlers[0].id,"input_ref":input,"grant_id":grant.resource_id}));
            let receipt = self.store.execute(command.clone()).await?;
            if self.store.execute(command).await? != receipt {
                return Err("idempotent receipt mismatch".into());
            }
            let resource = receipt.resource.ok_or("missing invocation receipt")?;
            let projection = self
                .store
                .read_projection(
                    &self.actor,
                    "PluginInvocation",
                    resource.resource_id.as_str(),
                )
                .await?;
            if projection.artifact_ref.classification != Classification::Confidential {
                return Err("projection classification downgraded".into());
            }
            let (_, bytes) = self
                .store
                .artifact_content(
                    &self.actor,
                    projection.artifact_ref.artifact_id.as_str(),
                    1024 * 1024,
                )
                .await?;
            let value: Value = serde_json::from_slice(&bytes)?;
            let output: ArtifactRef = serde_json::from_value(value["output_ref"].clone())?;
            let (_, bytes) = self
                .store
                .artifact_content(&self.actor, output.artifact_id.as_str(), 1024 * 1024)
                .await?;
            let output_value: String = serde_json::from_slice(&bytes)?;
            let expected = match manifest.id.as_str() {
                checksum_fixture::PLUGIN_ID => {
                    digest_bytes(b"Masonwing local native proof").to_string()
                }
                document_review_fixture::PLUGIN_ID => {
                    document_review_fixture::review_bytes(b"Masonwing local native proof")
                }
                _ => return Err("unexpected fixture domain".into()),
            };
            if output_value != expected || output.classification != Classification::Confidential {
                return Err("output mismatch".into());
            }
            results.push(json!({"plugin_id":manifest.id,"handler":manifest.handlers[0].id,"state":"SUCCEEDED","actual_output":output_value,
                "output_digest":output.digest,"classification":output.classification,"receipt_replay_equal":true}));
            locked.push(json!({"plugin_id":manifest.id,"artifact_digest":manifest.artifact_digest,"contract_version":manifest.contract_version}));
        }
        let lock = canonical_bytes(
            &json!({"schema_version":"1.0.0","product_id":"local.fixture.composition","plugins":locked}),
        )?;
        let lock_ref = self.reference(&lock, Classification::Internal);
        self.seed_artifact(&lock_ref, lock).await?;
        self.store
            .execute(self.command(
                "product.compose",
                json!({"product_id":"local.fixture.composition","plugin_lock_ref":lock_ref}),
            ))
            .await?;
        Ok(
            json!({"observed_at":chrono::Utc::now(),"kind":"APPLICATION_INTEGRATION","result":"PASS","postgres":"real runtime role with FORCE RLS",
            "object_storage":"real local MinIO immutable artifacts","execution":"real compiled native fixture handlers through HostPluginRuntime",
            "tenant_id":self.tenant,"fixtures":results,"exact_composition":"SUCCEEDED","model_calls":0,"external_provider_mutations":0,
            "authentication_scope":"operator-created synthetic actor; no HTTP/OIDC/browser or Temporal claim",
            "cleanup":"ephemeral tenant/database records removed; immutable test objects remain unreachable pending object GC"}),
        )
    }

    async fn cleanup(&self) -> Result {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        for table in [
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
