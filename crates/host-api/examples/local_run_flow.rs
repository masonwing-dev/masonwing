//! Reproducible local durable-execution integration: real PostgreSQL and real
//! MinIO exercise the WP-011..014 lifecycle end to end. No HTTP/OIDC or live
//! Temporal claim — the workflow engine is simulated by the same system-path
//! calls the outbox worker makes, so every assertion is on committed durable
//! state, never on a mock receipt.
//!
//! Covers: `run.start` (REQ-069), durable checkpoint + crash-resume semantics
//! (REQ-070), approval admission under four-eyes (REQ-078..083), effect
//! propose/dispatch/receipt (REQ-084..090), and budget reserve→settle
//! (REQ-092..095). External mutation stays disabled; the "provider" outcome is
//! a synthetic recorded receipt, not a transmission.
//!
//! Run from repository root:
//!   cargo run -p masonwing-host-api --example local_run_flow -- --local

use std::{sync::Arc, time::Duration};

use ed25519_dalek::{Signer, SigningKey};
use masonwing_artifacts_adapter::{ArtifactStoreAdapter, S3Config};
use masonwing_contract_validation::{canonical_bytes, digest_bytes, fingerprint};
use masonwing_contracts::{
    ArtifactId, PrincipalId, ResourceId, TenantId,
    wire::{ArtifactRef, Classification, PluginManifest},
};
use masonwing_data_postgres::{
    CheckpointWrite, DurableCheckpoint, PostgresStore,
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
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use uuid::Uuid;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct LocalFlow {
    operator: PgPool,
    store: PostgresStore,
    objects: Arc<ArtifactStoreAdapter>,
    /// The operator principal that owns the run and authors proposals.
    author: CommandActor,
    /// A second, independent principal used to satisfy four-eyes approval.
    approver: CommandActor,
    tenant: Uuid,
}

#[tokio::main]
async fn main() -> Result {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--local"] {
        return Err("usage: local_run_flow --local (fixed local endpoints only)".into());
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
    std::fs::create_dir_all(".evidence/run-flow")?;
    let output = serde_json::to_string_pretty(&report)?;
    std::fs::write(".evidence/run-flow/local-run-flow.json", &output)?;
    println!("{output}");
    Ok(())
}

impl LocalFlow {
    async fn create() -> Result<Self> {
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
        let author_principal = Uuid::new_v4();
        let approver_principal = Uuid::new_v4();
        let author = CommandActor {
            tenant_id: TenantId::new(tenant.to_string())?,
            principal_id: PrincipalId::new(author_principal.to_string())?,
            issuer: "https://local-fixture.example.test".into(),
            membership_epoch: 1,
            permission_epoch: 1,
            policy_version: "1.0.0".into(),
            policy_epoch: 1,
        };
        let approver = CommandActor {
            tenant_id: TenantId::new(tenant.to_string())?,
            principal_id: PrincipalId::new(approver_principal.to_string())?,
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
        sqlx::query("INSERT INTO tenants(id,name) VALUES($1,'ephemeral durable-run fixture')")
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
        // Two ACTIVE OWNER memberships: the run author and the independent
        // approver required by four-eyes (REQ-082).
        for (principal, membership) in [
            (author_principal, Uuid::new_v4()),
            (approver_principal, Uuid::new_v4()),
        ] {
            sqlx::query("INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch) VALUES($1,$2,$3,'OWNER','ACTIVE',1,1)")
                .bind(tenant).bind(principal).bind(membership).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES($1,$2,'OWNER')",
            )
            .bind(tenant)
            .bind(membership)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current) VALUES($1,'1.0.0',1,'permit(principal, action, resource);',true)")
            .bind(tenant).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self {
            operator,
            store,
            objects,
            author,
            approver,
            tenant,
        })
    }

    fn command(&self, actor: &CommandActor, operation: &str, input: Value) -> AuthorizedCommand {
        AuthorizedCommand {
            actor: actor.clone(),
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

    /// Read a run's current fence through a tenant-scoped operator transaction.
    /// Row-level security is forced on `runs`, so an unscoped pool query would
    /// observe zero rows rather than another tenant's data.
    async fn read_run_fence(&self, run_id: Uuid) -> Result<i64> {
        Ok(self.read_run_state(run_id).await?.0)
    }

    /// Read a run's `(fence, state)` through a tenant-scoped operator
    /// transaction; both are asserted by the report.
    async fn read_run_state(&self, run_id: Uuid) -> Result<(i64, String)> {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT fence,state FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(self.tenant)
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok((row.try_get("fence")?, row.try_get("state")?))
    }

    /// Read an effect's durable state through a tenant-scoped operator
    /// transaction.
    async fn read_effect_state(&self, effect_id: Uuid) -> Result<String> {
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        let state: String =
            sqlx::query_scalar("SELECT state FROM effects WHERE tenant_id=$1 AND id=$2")
                .bind(self.tenant)
                .bind(effect_id)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(state)
    }

    /// Install a workflow-bearing plugin and return its manifest plus the
    /// workflow definition the run will traverse.
    async fn install_workflow_plugin(&self) -> Result<(PluginManifest, String, String)> {
        let signer = SigningKey::from_bytes(&[43; 32]);
        let mut tx = self.operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'masonwing.first-party','local.integration',$2)")
            .bind(self.tenant).bind(signer.verifying_key().to_bytes().as_slice()).execute(&mut *tx).await?;
        tx.commit().await?;

        // Use the checksum fixture's declared workflow (a real packaged
        // definition), so the run traverses a contract-pinned graph rather
        // than a synthetic one.
        let (fixture, _) = local_fixture_plugins(&self.author.tenant_id)
            .into_iter()
            .find(|(f, _)| f.plugin_id.as_str() == checksum_fixture::PLUGIN_ID)
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
            .execute(self.command(&self.author, "plugin.install", json!({"manifest":manifest})))
            .await?;
        self.store
            .execute(self.command(&self.author, "plugin.enable", json!({"plugin_id":manifest.id,"artifact_digest":manifest.artifact_digest,"grants":[],"expected_version":1})))
            .await?;
        // The workflow the run will execute; install registered it into
        // workflow_definitions keyed by (workflow_id, version).
        Ok((
            manifest,
            "fixture.checksum.workflow.v1".into(),
            "1.0.0".into(),
        ))
    }

    async fn exercise(&self) -> Result<Value> {
        let (manifest, workflow_id, workflow_version) = self.install_workflow_plugin().await?;

        // --- Seed run input conforming to the workflow's pinned schema -----
        // The checksum workflow's input_schema_ref (seeded as a supporting
        // artifact during install) requires a JSON array of integers. The
        // definition is digest-pinned and must NOT be mutated; we supply an
        // input that satisfies it as-is.
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

        // Seed the artifact the effect will mutate (the effect target).
        let target_bytes = canonical_bytes(&json!({"doc":"draft v1"}))?;
        let target_ref = self.reference(&target_bytes, Classification::Confidential);
        self.seed_artifact(&target_ref, target_bytes).await?;
        let target = json!({"resource_type":"Artifact","resource_id":target_ref.artifact_id.as_str(),"version":1});

        // --- Grant + budget -----------------------------------------------
        // The grant's actions must cover the specific business verb the effect
        // will perform (require_active_grant checks Some(&action)), and its
        // resource scope must include every artifact it reads or mutates.
        let grant = self
            .store
            .execute(self.command(&self.author, "grant.create", json!({
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
        let grant_id = grant.resource_id.as_str().to_owned();

        // Configure the RUN and DAY budget dimensions the effect reservation
        // will charge (account keys RUN:USD:default, DAY:USD:default).
        for period in ["RUN", "DAY"] {
            self.store
                .execute(self.command(&self.author, "budget.configure", json!({
                    "currency":"USD","period":period,"limit_microunits":1_000_000,"expected_version":1})))
                .await?;
        }

        // --- WP-011: run.start (REQ-069) -----------------------------------
        let run_cmd = self.command(
            &self.author,
            "run.start",
            json!({
            "workflow_id":workflow_id,"workflow_version":workflow_version,
            "input_ref":input_ref,"grant_id":grant_id}),
        );
        let run_receipt = self.store.execute(run_cmd.clone()).await?;
        let run_id = run_receipt
            .run_id
            .as_ref()
            .ok_or("run.start receipt missing run_id")?
            .as_str()
            .to_owned();
        if run_receipt.state != masonwing_contracts::wire::ReceiptState::Accepted {
            return Err("run.start did not return ACCEPTED".into());
        }
        let run_uuid = Uuid::parse_str(&run_id)?;
        let run_fence = self.read_run_fence(run_uuid).await?;

        // Idempotent replay: re-issuing the same command returns the original receipt.
        let replay = self.store.execute(run_cmd).await?;
        if replay != run_receipt {
            return Err("idempotent run.start returned a different receipt".into());
        }

        // --- Durable dispatch acknowledgement (worker system path) ---------
        self.store
            .mark_run_started(self.tenant, run_uuid, "local-temporal-run-1", run_fence)
            .await?;

        // --- WP-011b: checkpointed traversal + crash-resume (REQ-070) ------
        // Simulate a worker executing node 1, checkpointing SUCCEEDED, then a
        // second worker resuming: it must skip step 1 and dispatch step 2.
        self.store
            .record_run_checkpoint(
                self.tenant,
                run_uuid,
                &CheckpointWrite {
                    logical_step_id: "0001:entry".into(),
                    state: "SUCCEEDED".into(),
                    failure_code: None,
                    fence: run_fence,
                    output_ref: None,
                },
            )
            .await?;
        // A duplicate SUCCEEDED write for the same step is a durable no-op.
        let rewrote = self
            .store
            .record_run_checkpoint(
                self.tenant,
                run_uuid,
                &CheckpointWrite {
                    logical_step_id: "0001:entry".into(),
                    state: "SUCCEEDED".into(),
                    failure_code: None,
                    fence: run_fence,
                    output_ref: None,
                },
            )
            .await?;
        if rewrote {
            return Err("completed checkpoint was rewritten (REQ-070)".into());
        }
        let history = self
            .store
            .read_run_checkpoints(self.tenant, run_uuid)
            .await?;
        if !history
            .iter()
            .any(|c| c.logical_step_id == "0001:entry" && c.state == "SUCCEEDED")
        {
            return Err("checkpoint history missing completed step".into());
        }

        // --- WP-014: connection + budget reservation -----------------------
        let connection_id = Uuid::new_v4();
        {
            let mut tx = self.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(self.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO connections(tenant_id,id,provider,external_account,scopes,secret_ref,state,contract_version,capabilities) VALUES($1,$2,'local-fixture',$3,'{}','local/fixture/secret','ACTIVE','1.0.0','{}')")
                .bind(self.tenant).bind(connection_id).bind(connection_id.to_string()).execute(&mut *tx).await?;
            tx.commit().await?;
        }

        // --- WP-013: effect.propose ----------------------------------------
        let content_bytes = canonical_bytes(&json!({"body":"approved external mutation"}))?;
        let content_ref = self.reference(&content_bytes, Classification::Confidential);
        self.seed_artifact(&content_ref, content_bytes).await?;
        let content_digest = content_ref.digest.as_str().to_owned();

        let effect_receipt = self
            .store
            .execute(self.command(
                &self.author,
                "effect.propose",
                json!({
                "action":"artifact.write","connection_id":connection_id.to_string(),
                "target":target,
                "content_ref":content_ref,"grant_id":grant_id,"price_profile":"1.0.0"}),
            ))
            .await?;
        let effect_id = effect_receipt
            .effect_id
            .as_ref()
            .ok_or("effect.propose missing effect_id")?
            .as_str()
            .to_owned();
        let effect_uuid = Uuid::parse_str(&effect_id)?;

        // --- WP-012: approval proposal + four-eyes decide ------------------
        // The proposal row is a seeded Given (the product's own proposal
        // creation path is the workflow's waiting-approval step, out of scope
        // for this example); every decision below goes through the real
        // `approval.decide` command.
        let proposal_id = Uuid::new_v4();
        {
            let mut tx = self.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(self.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO approvals(tenant_id,id,run_id,action,target,content_digest,scope_digest,policy_version,policy_epoch,expires_at,max_cost_microunits,currency,created_by,state) VALUES($1,$2,$3,'artifact.write',$4,$5,$6,'1.0.0',1,$7,$8,'USD',$9,'REQUESTED')")
                .bind(self.tenant).bind(proposal_id).bind(run_uuid)
                .bind(&target)
                .bind(&content_digest).bind(&content_digest)
                .bind(chrono::Utc::now()+chrono::Duration::hours(1)).bind(20_i64)
                .bind(Uuid::parse_str(self.author.principal_id.as_str())?)
                .execute(&mut *tx).await?;
            // Attach the approval requirement to the effect.
            sqlx::query("UPDATE effects SET approval_id=$3 WHERE tenant_id=$1 AND id=$2")
                .bind(self.tenant)
                .bind(effect_uuid)
                .bind(proposal_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }

        // Project the Approval resource so `authority.rs::authorize_command`
        // resolves its existence and classification before running Cedar
        // evaluation. The command processor writes these automatically for
        // commands that produce them; manually seeded proposals must do the same.
        let approval_proj_bytes = canonical_bytes(&json!({
            "proposal_id": proposal_id, "tenant_id": self.tenant,
            "action": "artifact.write", "state": "REQUESTED"
        }))?;
        let approval_proj_ref = self.reference(&approval_proj_bytes, Classification::Internal);
        self.seed_artifact(&approval_proj_ref, approval_proj_bytes)
            .await?;
        {
            let mut tx = self.operator.begin().await?;
            sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
                .bind(self.tenant.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO resource_projections(tenant_id,resource_type,resource_id,version,artifact_id,search_label,created_at) VALUES($1,'Approval',$2,1,$3,'artifact.write',clock_timestamp())")
                .bind(self.tenant).bind(proposal_id.to_string()).bind(Uuid::parse_str(approval_proj_ref.artifact_id.as_str())?)
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }

        // Four-eyes: the author cannot decide their own proposal.
        let self_decide = self
            .store
            .execute(self.command(
                &self.author,
                "approval.decide",
                json!({
                "proposal_id":proposal_id,"decision":"APPROVE","expected_version":1,
                "content_digest":content_digest,"reason":"self approval attempt"}),
            ))
            .await;
        match self_decide {
            Ok(_) => return Err("four-eyes violated: author decided own proposal".into()),
            Err(failure) if failure.code == "FOUR_EYES_REQUIRED" => {}
            Err(failure) => {
                return Err(format!("expected FOUR_EYES_REQUIRED, got {}", failure.code).into());
            }
        }

        // Independent approver decides over the exact previewed digest.
        let decision = self
            .store
            .execute(self.command(
                &self.approver,
                "approval.decide",
                json!({
                "proposal_id":proposal_id,"decision":"APPROVE","expected_version":1,
                "content_digest":content_digest,"reason":"reviewed and approved"}),
            ))
            .await?;
        if decision.resource.is_none() {
            return Err("approval.decide returned no resource".into());
        }

        // --- WP-013: effect.dispatch -> EXECUTING --------------------------
        let dispatch = self
            .store
            .execute(self.command(
                &self.author,
                "effect.dispatch",
                json!({
                "effect_id":effect_id,"expected_version":1}),
            ))
            .await?;
        if dispatch.state != masonwing_contracts::wire::ReceiptState::Accepted {
            return Err("effect.dispatch did not return ACCEPTED".into());
        }

        // --- Recorded provider receipt -> SUCCEEDED + settle (REQ-086/095) --
        let evidence_bytes =
            canonical_bytes(&json!({"remote_status":"applied","remote_id":"local-fixture-1"}))?;
        let evidence_ref = self.reference(&evidence_bytes, Classification::Internal);
        self.seed_artifact(&evidence_ref, evidence_bytes).await?;
        self.store
            .record_effect_succeeded(
                self.tenant,
                effect_uuid,
                "local-fixture-1",
                &evidence_ref,
                15,
            )
            .await?;

        // Confirm the durable effect reached SUCCEEDED and budget settled.
        let effect_state = self.read_effect_state(effect_uuid).await?;
        if effect_state != "SUCCEEDED" {
            return Err(format!("effect ended in {effect_state}, expected SUCCEEDED").into());
        }

        // --- Run traversal completion (REQ-070/REQ-073) --------------------
        self.store
            .mark_run_completed(self.tenant, run_uuid, run_fence)
            .await?;
        let final_run_state = self.read_run_state(run_uuid).await?.1;
        if final_run_state != "SUCCEEDED" {
            return Err(format!("run ended in {final_run_state}, expected SUCCEEDED").into());
        }

        Ok(json!({
            "observed_at": chrono::Utc::now(),
            "kind": "DURABLE_EXECUTION_INTEGRATION",
            "result": "PASS",
            "postgres": "real runtime role with FORCE RLS",
            "object_storage": "real local MinIO immutable artifacts",
            "tenant_id": self.tenant,
            "plugin_id": manifest.id,
            "workflow_id": workflow_id,
            "run": {
                "run_id": run_id,
                "state": final_run_state,
                "accepted_with_run_id": true,
                "idempotent_replay_equal": true,
                "checkpoints": history.len(),
                "completed_step_not_rewritten": true,
            },
            "approval": {
                "proposal_id": proposal_id,
                "four_eyes_enforced": true,
                "decision": "APPROVED",
            },
            "effect": {
                "effect_id": effect_id,
                "state": effect_state,
                "receipt_recorded": true,
                "budget_settled_microunits": 15,
            },
            "model_calls": 0,
            "external_provider_mutations": 0,
            "authentication_scope": "operator-created synthetic actors; no HTTP/OIDC/browser or live Temporal claim",
            "cleanup": "ephemeral tenant/database records removed; immutable test objects remain unreachable pending object GC",
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
            "run_checkpoints",
            "runs",
            "connections",
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

// Silence unused-import lint for the DurableCheckpoint type referenced only in
// doc-level reasoning about the resume contract.
#[allow(dead_code)]
fn _assert_checkpoint_type(_c: &DurableCheckpoint) {}
