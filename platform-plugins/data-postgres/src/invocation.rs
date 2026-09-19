//! Synchronous, effect-free plugin invocation through a host-owned runtime port.
//! Current authorization is held until the pure result and receipt commit.

use std::collections::BTreeMap;

use masonwing_contract_validation::{ArtifactSchema, canonical_bytes, strict_json};
use masonwing_contracts::{Action, GrantId, wire::ArtifactRef};
use masonwing_kernel::runtime::{AuthorizedCommand, CommandFailure, PurePluginInvocation};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{
    Mutation, PostgresStore, Tx, database_failure, field, parse_uuid, resource_id, resource_ref,
};

const MAX_IO_BYTES: usize = 4 * 1024 * 1024;
const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;

impl PostgresStore {
    pub(crate) async fn invoke_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let runtime = self
            .plugin_runtime
            .as_ref()
            .ok_or_else(|| CommandFailure::unavailable("PLUGIN_RUNTIME_UNAVAILABLE"))?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let plugin: String = field(&command.input, "plugin_id")?;
        let row = sqlx::query("SELECT id,state,version,artifact_digest,granted_capabilities FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2 FOR SHARE")
            .bind(tenant).bind(&plugin).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        match row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            .as_str()
        {
            "ENABLED" => {}
            "DRAINING" => return Err(CommandFailure::precondition("PLUGIN_DRAINING")),
            "REVOKED" => return Err(CommandFailure::precondition("PLUGIN_REVOKED")),
            _ => return Err(CommandFailure::precondition("PLUGIN_NOT_ENABLED")),
        }
        let installation = resource_ref(
            "PluginInstallation",
            row.try_get("id").map_err(database_failure)?,
            row.try_get("version").map_err(database_failure)?,
        );
        let digest: String = row.try_get("artifact_digest").map_err(database_failure)?;
        let manifest = self.stored_manifest(tx, tenant, &plugin, &digest).await?;
        let handler_id: String = field(&command.input, "handler")?;
        let handler = manifest
            .handlers
            .iter()
            .find(|h| h.id == handler_id)
            .cloned()
            .ok_or_else(CommandFailure::not_found)?;
        if !matches!(handler.effects.as_str(), "DETERMINISTIC" | "READ_ONLY") {
            return Err(CommandFailure::precondition("EFFECT_BROKER_REQUIRED"));
        }
        self.require_dispatch_enabled(tx, &command.actor, Some(&plugin), None)
            .await?;
        let grant_id: GrantId = field(&command.input, "grant_id")?;
        let grant = self
            .require_active_grant(
                tx,
                &command.actor,
                parse_uuid(grant_id.as_str())?,
                Some("plugin.invoke"),
                Some(&installation),
            )
            .await?;
        let input_ref: ArtifactRef = field(&command.input, "input_ref")?;
        if input_ref.tenant_id != command.actor.tenant_id {
            return Err(CommandFailure::not_found());
        }
        let metadata = self
            .artifact_metadata_in(tx, &command.actor, input_ref.artifact_id.as_str())
            .await?;
        if metadata.reference != input_ref {
            return Err(CommandFailure::precondition("ARTIFACT_REFERENCE_MISMATCH"));
        }
        let input_id = parse_uuid(input_ref.artifact_id.as_str())?;
        let version: i32 =
            sqlx::query_scalar("SELECT version FROM artifacts WHERE tenant_id=$1 AND id=$2")
                .bind(tenant)
                .bind(input_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(database_failure)?;
        if !grant.actions.iter().any(|a| a == "artifact.read")
            || !grant
                .resources
                .contains(&resource_ref("Artifact", input_id, version))
        {
            return Err(CommandFailure::denied("CAPABILITY_DENIED"));
        }
        let can_transform: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifact_rights WHERE tenant_id=$1 AND artifact_id=$2 AND state='ACTIVE' AND allow_transform AND (expires_at IS NULL OR expires_at>clock_timestamp()))")
            .bind(tenant).bind(input_id).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if !can_transform {
            return Err(CommandFailure::denied("SOURCE_RIGHTS_DENIED"));
        }
        let installation_caps: Vec<String> = row
            .try_get("granted_capabilities")
            .map_err(database_failure)?;
        let policy = self.read_policy(tx, &command.actor).await?;
        let effective: Vec<Action> = installation_caps
            .iter()
            .filter(|action| grant.actions.contains(action))
            .filter(|action| {
                policy.permits(
                    action,
                    "PluginInstallation",
                    installation.resource_id.as_str(),
                    BTreeMap::from([
                        ("plugin_id".into(), json!(plugin)),
                        ("version".into(), json!(installation.version)),
                        ("state".into(), json!("ENABLED")),
                    ]),
                )
            })
            .map(|action| {
                Action::new(action.clone())
                    .map_err(|_| CommandFailure::unavailable("CAPABILITY_INTEGRITY"))
            })
            .collect::<Result<_, _>>()?;
        if handler
            .required_capabilities
            .iter()
            .any(|required| !effective.contains(required))
        {
            return Err(CommandFailure::denied("CAPABILITY_DENIED"));
        }
        // Trust and storage reads occur after grant/rights denials, so an invalid
        // scope never causes guest compilation or reads of confidential bytes.
        self.verify_publisher_and_package(tx, &command.actor, &manifest)
            .await?;
        let package_ref = self
            .package_artifact(
                tx,
                &command.actor,
                &manifest.artifact_digest,
                MAX_PACKAGE_BYTES,
            )
            .await?;
        let package = self
            .load_artifact_ref(tx, &command.actor, &package_ref, MAX_PACKAGE_BYTES)
            .await?;
        let input_schema = self
            .load_artifact_ref(tx, &command.actor, &handler.input_schema_ref, 256 * 1024)
            .await?;
        let output_schema = self
            .load_artifact_ref(tx, &command.actor, &handler.output_schema_ref, 256 * 1024)
            .await?;
        let input_schema = ArtifactSchema::compile(&input_schema)
            .map_err(|_| CommandFailure::precondition("PLUGIN_SCHEMA_INVALID"))?;
        let output_schema = ArtifactSchema::compile(&output_schema)
            .map_err(|_| CommandFailure::precondition("PLUGIN_SCHEMA_INVALID"))?;
        let input = self
            .load_artifact_ref(tx, &command.actor, &input_ref, MAX_IO_BYTES)
            .await?;
        input_schema
            .validate_bytes(&input, MAX_IO_BYTES)
            .map_err(|_| CommandFailure::invalid("PLUGIN_INPUT_INVALID"))?;
        // SDK byte payloads are canonical JSON instances. Equivalent JSON number
        // representations (e.g. 97.0) must not pass schema validation and then
        // unexpectedly fail a native integer deserializer.
        let input = canonical_bytes(
            &strict_json(&input).map_err(|_| CommandFailure::invalid("PLUGIN_INPUT_INVALID"))?,
        )
        .map_err(|_| CommandFailure::invalid("PLUGIN_INPUT_INVALID"))?;
        let id = Uuid::new_v4();
        let output = runtime
            .invoke(PurePluginInvocation {
                invocation_id: resource_id(id),
                tenant_id: command.actor.tenant_id.clone(),
                manifest,
                handler,
                grant_id,
                granted_capabilities: effective,
                artifact_bytes: package,
                input,
            })
            .await?;
        self.require_active_grant(
            tx,
            &command.actor,
            grant.id,
            Some("plugin.invoke"),
            Some(&installation),
        )
        .await?;
        let rights_still_current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifact_rights WHERE tenant_id=$1 AND artifact_id=$2 AND state='ACTIVE' AND allow_transform AND (expires_at IS NULL OR expires_at>clock_timestamp()))")
            .bind(tenant).bind(input_id).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if !rights_still_current {
            return Err(CommandFailure::denied("SOURCE_RIGHTS_DENIED"));
        }
        output_schema
            .validate_bytes(&output.bytes, MAX_IO_BYTES)
            .map_err(|_| CommandFailure::precondition("PLUGIN_OUTPUT_INVALID"))?;
        let output = canonical_bytes(
            &strict_json(&output.bytes)
                .map_err(|_| CommandFailure::precondition("PLUGIN_OUTPUT_INVALID"))?,
        )
        .map_err(|_| CommandFailure::precondition("PLUGIN_OUTPUT_INVALID"))?;
        let output_ref = self
            .persist_artifact(
                tx,
                &command.actor,
                output,
                "application/json",
                input_ref.classification,
            )
            .await?;
        let output_id = parse_uuid(output_ref.artifact_id.as_str())?;
        sqlx::query(
            "INSERT INTO artifact_lineage(tenant_id,source_id,derived_id) VALUES($1,$2,$3)",
        )
        .bind(tenant)
        .bind(input_id)
        .bind(output_id)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        // Derived bytes never acquire AI/public-redistribution rights absent from
        // the input, and their permission expires with the source's evidence.
        sqlx::query("INSERT INTO artifact_rights(tenant_id,artifact_id,rights_id,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,allowed_provider_profiles,expires_at,evidence_ref,state) SELECT tenant_id,$3,$4,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,allowed_provider_profiles,expires_at,evidence_ref,'ACTIVE' FROM artifact_rights WHERE tenant_id=$1 AND artifact_id=$2")
            .bind(tenant).bind(input_id).bind(output_id).bind(Uuid::new_v4()).execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "PluginInvocation",
            id,
            1,
            json!({
                "invocation_id":id,"plugin_id":plugin,"plugin_digest":digest,"handler":handler_id,
                "grant_id":grant.id,"grant_fence":grant.fence,"input_ref":input_ref,"output_ref":output_ref,
                "state":"SUCCEEDED","version":1,
            }),
            format!("{plugin} {handler_id}"),
        ))
    }
}
