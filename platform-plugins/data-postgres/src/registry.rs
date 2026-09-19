//! Tenant registry and lifecycle commands. Publisher trust is operator-owned;
//! command inputs cannot supply trusted keys or execute arbitrary migrations.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use masonwing_contract_validation::{ArtifactSchema, MAX_COMMAND_BYTES, digest_bytes, strict_json};
use masonwing_contracts::{
    ArtifactId, Digest, PluginId,
    wire::{ArtifactRef, ExecutionClass, PluginManifest, WorkflowDefinition},
};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, require_expected_version,
};
use semver::Version;
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use crate::{
    registry_validation::{self as checks, PluginLock},
    store::{
        Mutation, PostgresStore, ProjectionChange, Tx, database_failure, field, parse_uuid,
        resource_ref,
    },
};

const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;

impl PostgresStore {
    pub(crate) async fn install_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let manifest: PluginManifest = field(&command.input, "manifest")?;
        let workflows = self
            .verify_installation(tx, &command.actor, &manifest)
            .await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        self.save_plugin_version(tx, tenant, &manifest, &workflows)
            .await?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO plugin_installations(tenant_id,id,plugin_id,artifact_digest,state) VALUES($1,$2,$3,$4,'INSTALLED_DISABLED')")
            .bind(tenant).bind(id).bind(manifest.id.as_str()).bind(manifest.artifact_digest.as_str())
            .execute(&mut **tx).await.map_err(database_failure)?;
        let mut result = self.installation_projection(tx, tenant, id).await?;
        result.changes.push(manifest_projection(&manifest)?);
        Ok(result)
    }

    pub(crate) async fn enable_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let (id, current) = self.current_installation(tx, command).await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let next = checked_version(command, &current)?;
        let state: String = current.try_get("state").map_err(database_failure)?;
        if !matches!(state.as_str(), "INSTALLED_DISABLED" | "DISABLED") {
            return Err(illegal());
        }
        let digest: String = field(&command.input, "artifact_digest")?;
        if current
            .try_get::<String, _>("artifact_digest")
            .map_err(database_failure)?
            != digest
        {
            return Err(CommandFailure::conflict("PLUGIN_DIGEST_MISMATCH"));
        }
        let manifest = self
            .stored_manifest(
                tx,
                tenant,
                &field::<String>(&command.input, "plugin_id")?,
                &digest,
            )
            .await?;
        self.verify_publisher_and_package(tx, &command.actor, &manifest)
            .await?;
        self.require_enabled_dependencies(tx, tenant, &manifest)
            .await?;
        self.require_dispatch_enabled(tx, &command.actor, Some(manifest.id.as_str()), None)
            .await?;
        let grants: Vec<String> = field(&command.input, "grants")?;
        let unique: BTreeSet<_> = grants.iter().collect();
        let policy = self.read_policy(tx, &command.actor).await?;
        if unique.len() != grants.len()
            || grants.iter().any(|grant| {
                !manifest
                    .requested_capabilities
                    .iter()
                    .any(|a| a.as_str() == grant)
                    || !policy.permits(grant, "Tenant", &tenant.to_string(), BTreeMap::new())
            })
        {
            return Err(CommandFailure::denied("CAPABILITY_DENIED"));
        }
        sqlx::query("UPDATE plugin_installations SET state='ENABLED',granted_capabilities=$3,version=$4,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).bind(grants).bind(next).execute(&mut **tx).await.map_err(database_failure)?;
        self.installation_projection(tx, tenant, id).await
    }

    pub(crate) async fn disable_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        require_reason(command)?;
        let (id, current) = self.current_installation(tx, command).await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let mut next = checked_version(command, &current)?;
        let state: String = current.try_get("state").map_err(database_failure)?;
        let plugin: String = current.try_get("plugin_id").map_err(database_failure)?;
        let active = self.has_active_plugin_runs(tx, tenant, &plugin).await?;
        if !matches!(state.as_str(), "ENABLED" | "DRAINING") || (state == "DRAINING" && active) {
            return Err(illegal());
        }
        let mut changes = Vec::new();
        if state == "ENABLED" {
            sqlx::query("UPDATE plugin_installations SET state='DRAINING',version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
                .bind(tenant).bind(id).bind(next).execute(&mut **tx).await.map_err(database_failure)?;
            changes.extend(self.installation_projection(tx, tenant, id).await?.changes);
            if !active {
                next = next.checked_add(1).ok_or_else(illegal)?;
            }
        }
        if !active {
            sqlx::query("UPDATE plugin_installations SET state='DISABLED',version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
                .bind(tenant).bind(id).bind(next).execute(&mut **tx).await.map_err(database_failure)?;
            changes.extend(self.installation_projection(tx, tenant, id).await?.changes);
        }
        let mut result = self.installation_projection(tx, tenant, id).await?;
        result.changes = changes;
        Ok(result)
    }

    pub(crate) async fn upgrade_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let (id, current) = self.current_installation(tx, command).await?;
        let next = checked_version(command, &current)?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let manifest: PluginManifest = field(&command.input, "new_manifest")?;
        let plugin: String = current.try_get("plugin_id").map_err(database_failure)?;
        if manifest.id.as_str() != plugin {
            return Err(checks::invalid());
        }
        let state: String = current.try_get("state").map_err(database_failure)?;
        if !matches!(
            state.as_str(),
            "ENABLED" | "DISABLED" | "INSTALLED_DISABLED"
        ) {
            return Err(illegal());
        }
        let old = self
            .stored_manifest(
                tx,
                tenant,
                &plugin,
                &current
                    .try_get::<String, _>("artifact_digest")
                    .map_err(database_failure)?,
            )
            .await?;
        if Version::parse(&manifest.version).map_err(|_| checks::invalid())?
            <= Version::parse(&old.version).map_err(|_| checks::invalid())?
        {
            return Err(CommandFailure::precondition("ROLLBACK_UNSAFE"));
        }
        let workflows = self
            .verify_installation(tx, &command.actor, &manifest)
            .await?;
        if state == "ENABLED" {
            self.require_enabled_dependencies(tx, tenant, &manifest)
                .await?;
        }
        let current_grants: Vec<String> = current
            .try_get("granted_capabilities")
            .map_err(database_failure)?;
        let grants: Vec<_> = current_grants
            .into_iter()
            .filter(|g| {
                manifest
                    .requested_capabilities
                    .iter()
                    .any(|a| a.as_str() == g)
            })
            .collect();
        self.save_plugin_version(tx, tenant, &manifest, &workflows)
            .await?;
        // Existing runs continue to reference their original plugin_versions row.
        // An upgrade neither broadens grants nor rewrites a pinned run digest.
        sqlx::query("UPDATE plugin_installations SET artifact_digest=$3,granted_capabilities=$4,version=$5,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).bind(manifest.artifact_digest.as_str()).bind(grants).bind(next)
            .execute(&mut **tx).await.map_err(database_failure)?;
        let mut result = self.installation_projection(tx, tenant, id).await?;
        result.changes.push(manifest_projection(&manifest)?);
        Ok(result)
    }

    pub(crate) async fn revoke_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let reason = require_reason(command)?;
        let (id, current) = self.current_installation(tx, command).await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let plugin: String = field(&command.input, "plugin_id")?;
        let digest: String = field(&command.input, "artifact_digest")?;
        self.stored_manifest(tx, tenant, &plugin, &digest).await?;
        sqlx::query("INSERT INTO plugin_revocations(tenant_id,plugin_id,artifact_digest,reason,revoked_by) VALUES($1,$2,$3,$4,$5)")
            .bind(tenant).bind(&plugin).bind(&digest).bind(reason).bind(parse_uuid(command.actor.principal_id.as_str())?)
            .execute(&mut **tx).await.map_err(database_failure)?;
        let next = current
            .try_get::<i32, _>("version")
            .map_err(database_failure)?
            .checked_add(1)
            .ok_or_else(illegal)?;
        sqlx::query("UPDATE plugin_installations SET state=CASE WHEN artifact_digest=$3 AND state<>'UNINSTALLED' THEN 'REVOKED' ELSE state END,granted_capabilities=CASE WHEN artifact_digest=$3 THEN '{}'::text[] ELSE granted_capabilities END,revocation_epoch=revocation_epoch+1,version=$4,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).bind(&digest).bind(next).execute(&mut **tx).await.map_err(database_failure)?;
        let mut result = self.installation_projection(tx, tenant, id).await?;
        let runs = sqlx::query("UPDATE runs SET state=CASE WHEN state='RUNNING' THEN 'BLOCKED' ELSE state END,dispatch_state='BLOCKED',failure_code='PLUGIN_REVOKED',fence=fence+1,version=version+1,updated_at=clock_timestamp() WHERE tenant_id=$1 AND plugin_id=$2 AND plugin_digest=$3 AND state NOT IN ('SUCCEEDED','FAILED','CANCELLED') RETURNING *")
            .bind(tenant).bind(plugin).bind(digest).fetch_all(&mut **tx).await.map_err(database_failure)?;
        for row in runs {
            result.changes.push(run_projection(&row, tenant)?);
        }
        Ok(result)
    }

    pub(crate) async fn uninstall_plugin(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let (id, current) = self.current_installation(tx, command).await?;
        let next = checked_version(command, &current)?;
        let state: String = current.try_get("state").map_err(database_failure)?;
        if !matches!(state.as_str(), "INSTALLED_DISABLED" | "DISABLED") {
            return Err(illegal());
        }
        if field::<String>(&command.input, "data_policy")? != "RETAIN" {
            return Err(CommandFailure::precondition("EXPORT_REQUIRED"));
        }
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let plugin: String = current.try_get("plugin_id").map_err(database_failure)?;
        if self.has_active_plugin_runs(tx, tenant, &plugin).await? {
            return Err(CommandFailure::precondition("PLUGIN_HAS_ACTIVE_RUNS"));
        }
        let mut selected = self.selected_manifests(tx, tenant).await?;
        selected.remove(&PluginId::new(plugin).map_err(|_| checks::invalid())?);
        checks::resolve_manifests(&selected)?;
        sqlx::query("UPDATE plugin_installations SET state='UNINSTALLED',granted_capabilities='{}',revocation_epoch=revocation_epoch+1,version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).bind(next).execute(&mut **tx).await.map_err(database_failure)?;
        self.installation_projection(tx, tenant, id).await
    }

    pub(crate) async fn compose_product(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let product: String = field(&command.input, "product_id")?;
        let reference: ArtifactRef = field(&command.input, "plugin_lock_ref")?;
        let bytes = self
            .load_artifact_ref(tx, &command.actor, &reference, MAX_COMMAND_BYTES)
            .await?;
        let lock: PluginLock = serde_json::from_value(
            strict_json(&bytes).map_err(|_| CommandFailure::invalid("PLUGIN_LOCK_INVALID"))?,
        )
        .map_err(|_| CommandFailure::invalid("PLUGIN_LOCK_INVALID"))?;
        checks::validate_lock(&lock, &product)?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let mut selected = BTreeMap::new();
        for pinned in &lock.plugins {
            let row = sqlx::query("SELECT artifact_digest,state FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2")
                .bind(tenant).bind(pinned.plugin_id.as_str()).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
            if row
                .try_get::<String, _>("state")
                .map_err(database_failure)?
                != "ENABLED"
            {
                return Err(CommandFailure::precondition("PLUGIN_NOT_ENABLED"));
            }
            if row
                .try_get::<String, _>("artifact_digest")
                .map_err(database_failure)?
                != pinned.artifact_digest.as_str()
            {
                return Err(CommandFailure::conflict("PLUGIN_LOCK_MISMATCH"));
            }
            let manifest = self
                .stored_manifest(
                    tx,
                    tenant,
                    pinned.plugin_id.as_str(),
                    pinned.artifact_digest.as_str(),
                )
                .await?;
            if manifest.contract_version != pinned.contract_version {
                return Err(CommandFailure::precondition("CONTRACT_UNSUPPORTED"));
            }
            self.verify_publisher_and_package(tx, &command.actor, &manifest)
                .await?;
            selected.insert(manifest.id.clone(), manifest);
        }
        let order = checks::resolve_manifests(&selected)?;
        let enabled: Vec<_> = order.iter().map(ToString::to_string).collect();
        let id = Uuid::new_v4();
        // The immutable command carries no expected_version: creating a product
        // must not silently overwrite an existing composition with another lock.
        sqlx::query("INSERT INTO product_compositions(tenant_id,id,product_id,lock_artifact_id,lock_digest,enabled_plugins) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(tenant).bind(id).bind(&product).bind(parse_uuid(reference.artifact_id.as_str())?).bind(reference.digest.as_str()).bind(&enabled)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "ProductComposition",
            id,
            1,
            json!({"composition_id":id,"product_id":product,"plugin_lock_ref":reference,"enabled_plugins":enabled,"version":1}),
            product,
        ))
    }

    async fn verify_installation(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        manifest: &PluginManifest,
    ) -> Result<Vec<WorkflowDefinition>, CommandFailure> {
        checks::validate_manifest(manifest, &actor.tenant_id, &self.known_capabilities)?;
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let mut selected = self.selected_manifests(tx, tenant).await?;
        selected.insert(manifest.id.clone(), manifest.clone());
        checks::resolve_manifests(&selected)?;
        if !manifest.migrations.is_empty() {
            return Err(CommandFailure::precondition("PLUGIN_MIGRATION_UNQUALIFIED"));
        }
        self.verify_publisher_and_package(tx, actor, manifest)
            .await?;
        let mut references: BTreeMap<String, ArtifactRef> = BTreeMap::new();
        for reference in manifest
            .handlers
            .iter()
            .flat_map(|h| [&h.input_schema_ref, &h.output_schema_ref])
            .chain(
                manifest
                    .tools
                    .iter()
                    .flat_map(|t| [&t.input_schema_ref, &t.output_schema_ref]),
            )
            .chain(&manifest.data_contracts)
        {
            if let Some(previous) =
                references.insert(reference.artifact_id.to_string(), reference.clone())
                && previous != *reference
            {
                return Err(CommandFailure::precondition("ARTIFACT_REFERENCE_MISMATCH"));
            }
        }
        for reference in references.values() {
            let bytes = self
                .load_artifact_ref(tx, actor, reference, MAX_COMMAND_BYTES)
                .await?;
            ArtifactSchema::compile(&bytes)
                .map_err(|_| CommandFailure::precondition("PLUGIN_SCHEMA_INVALID"))?;
        }
        let mut workflows = Vec::new();
        let mut workflow_ids = BTreeSet::new();
        for reference in &manifest.workflows {
            let bytes = self
                .load_artifact_ref(tx, actor, reference, MAX_COMMAND_BYTES)
                .await?;
            let value = strict_json(&bytes)
                .map_err(|_| CommandFailure::precondition("WORKFLOW_INVALID"))?;
            let definition: WorkflowDefinition = serde_json::from_value(value)
                .map_err(|_| CommandFailure::precondition("WORKFLOW_INVALID"))?;
            checks::validate_workflow(&definition, manifest)?;
            if !workflow_ids.insert((definition.id.clone(), definition.version.clone())) {
                return Err(CommandFailure::precondition("WORKFLOW_INVALID"));
            }
            for reference in [&definition.input_schema_ref, &definition.output_schema_ref]
                .into_iter()
                .chain(
                    definition
                        .nodes
                        .iter()
                        .flat_map(|n| [&n.input_schema_ref, &n.output_schema_ref]),
                )
            {
                let bytes = self
                    .load_artifact_ref(tx, actor, reference, MAX_COMMAND_BYTES)
                    .await?;
                ArtifactSchema::compile(&bytes)
                    .map_err(|_| CommandFailure::precondition("PLUGIN_SCHEMA_INVALID"))?;
            }
            workflows.push(definition);
        }
        Ok(workflows)
    }

    pub(crate) async fn verify_publisher_and_package(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        manifest: &PluginManifest,
    ) -> Result<(), CommandFailure> {
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let revoked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plugin_revocations WHERE tenant_id=$1 AND plugin_id=$2 AND artifact_digest=$3)")
            .bind(tenant).bind(manifest.id.as_str()).bind(manifest.artifact_digest.as_str()).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if revoked {
            return Err(CommandFailure::precondition("PLUGIN_REVOKED"));
        }
        let signature = sqlx::query("SELECT s.publisher_id,s.artifact_digest,s.manifest_digest,s.signature,k.public_key FROM artifact_signatures s JOIN publisher_keys k ON k.tenant_id=s.tenant_id AND k.publisher_id=s.publisher_id AND k.key_id=s.key_id WHERE s.tenant_id=$1 AND s.id=$2 AND k.revoked_at IS NULL")
            .bind(tenant).bind(&manifest.signature_ref).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(|| CommandFailure::precondition("PUBLISHER_UNTRUSTED"))?;
        if signature
            .try_get::<String, _>("publisher_id")
            .map_err(database_failure)?
            != manifest.publisher_id
            || signature
                .try_get::<String, _>("artifact_digest")
                .map_err(database_failure)?
                != manifest.artifact_digest.as_str()
            || signature
                .try_get::<String, _>("manifest_digest")
                .map_err(database_failure)?
                != checks::manifest_digest(manifest)?.as_str()
        {
            return Err(CommandFailure::precondition("SIGNATURE_INVALID"));
        }
        checks::verify_signature(
            manifest,
            &signature
                .try_get::<Vec<u8>, _>("public_key")
                .map_err(database_failure)?,
            &signature
                .try_get::<Vec<u8>, _>("signature")
                .map_err(database_failure)?,
        )?;
        if manifest.execution_class == ExecutionClass::TrustedNative
            && !self.native_packages.contains(&(
                manifest.id.to_string(),
                manifest.artifact_digest.to_string(),
            ))
        {
            return Err(CommandFailure::precondition(
                "NATIVE_PLUGIN_NOT_ALLOWLISTED",
            ));
        }
        self.package_artifact(tx, actor, &manifest.artifact_digest, MAX_PACKAGE_BYTES)
            .await?;
        self.package_artifact(tx, actor, &manifest.sbom_digest, MAX_COMMAND_BYTES)
            .await?;
        Ok(())
    }

    pub(crate) async fn package_artifact(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        digest: &Digest,
        maximum: usize,
    ) -> Result<ArtifactRef, CommandFailure> {
        let row = sqlx::query("SELECT id,classification,schema_version FROM artifacts a WHERE tenant_id=$1 AND digest=$2 AND state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM deletion_tombstones d WHERE d.tenant_id=a.tenant_id AND d.resource_type='Artifact' AND d.resource_id=a.id::text) ORDER BY created_at,id LIMIT 1")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(digest.as_str()).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let reference = ArtifactRef {
            artifact_id: ArtifactId::new(
                row.try_get::<Uuid, _>("id")
                    .map_err(database_failure)?
                    .to_string(),
            )
            .map_err(|_| checks::invalid())?,
            tenant_id: actor.tenant_id.clone(),
            digest: digest.clone(),
            schema_version: row.try_get("schema_version").map_err(database_failure)?,
            classification: serde_json::from_value(json!(
                row.try_get::<String, _>("classification")
                    .map_err(database_failure)?
            ))
            .map_err(|_| checks::invalid())?,
        };
        self.load_artifact_ref(tx, actor, &reference, maximum)
            .await?;
        Ok(reference)
    }

    async fn selected_manifests(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
    ) -> Result<BTreeMap<PluginId, PluginManifest>, CommandFailure> {
        let values: Vec<Value> = sqlx::query_scalar("SELECT v.manifest FROM plugin_installations i JOIN plugin_versions v ON v.tenant_id=i.tenant_id AND v.plugin_id=i.plugin_id AND v.artifact_digest=i.artifact_digest WHERE i.tenant_id=$1 AND i.state NOT IN ('UNINSTALLED','REVOKED') ORDER BY i.plugin_id LIMIT 513")
            .bind(tenant).fetch_all(&mut **tx).await.map_err(database_failure)?;
        values
            .into_iter()
            .map(|v| {
                serde_json::from_value::<PluginManifest>(v)
                    .map(|m| (m.id.clone(), m))
                    .map_err(|_| checks::invalid())
            })
            .collect()
    }

    pub(crate) async fn stored_manifest(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        plugin: &str,
        digest: &str,
    ) -> Result<PluginManifest, CommandFailure> {
        let value: Value = sqlx::query_scalar("SELECT manifest FROM plugin_versions WHERE tenant_id=$1 AND plugin_id=$2 AND artifact_digest=$3")
            .bind(tenant).bind(plugin).bind(digest).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        serde_json::from_value(value).map_err(|_| checks::invalid())
    }

    async fn save_plugin_version(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        manifest: &PluginManifest,
        workflows: &[WorkflowDefinition],
    ) -> Result<(), CommandFailure> {
        sqlx::query("INSERT INTO plugin_versions(tenant_id,plugin_id,artifact_digest,manifest,contract_version,plugin_version,signature_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(tenant).bind(manifest.id.as_str()).bind(manifest.artifact_digest.as_str()).bind(serde_json::to_value(manifest).map_err(|_| checks::invalid())?)
            .bind(&manifest.contract_version).bind(&manifest.version).bind(&manifest.signature_ref).execute(&mut **tx).await.map_err(database_failure)?;
        for definition in workflows {
            sqlx::query("INSERT INTO workflow_definitions(tenant_id,workflow_id,workflow_version,plugin_id,plugin_digest,definition_digest,definition) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(tenant).bind(&definition.id).bind(&definition.version).bind(manifest.id.as_str()).bind(manifest.artifact_digest.as_str())
                .bind(definition.digest.as_str()).bind(serde_json::to_value(definition).map_err(|_| checks::invalid())?).execute(&mut **tx).await.map_err(database_failure)?;
        }
        Ok(())
    }

    async fn current_installation(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<(Uuid, PgRow), CommandFailure> {
        let plugin: String = field(&command.input, "plugin_id")?;
        let row = sqlx::query(
            "SELECT * FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2 FOR UPDATE",
        )
        .bind(parse_uuid(command.actor.tenant_id.as_str())?)
        .bind(plugin)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        Ok((row.try_get("id").map_err(database_failure)?, row))
    }

    async fn require_enabled_dependencies(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        manifest: &PluginManifest,
    ) -> Result<(), CommandFailure> {
        for dependency in &manifest.dependencies {
            let row = sqlx::query("SELECT i.state,v.contract_version FROM plugin_installations i JOIN plugin_versions v ON v.tenant_id=i.tenant_id AND v.plugin_id=i.plugin_id AND v.artifact_digest=i.artifact_digest WHERE i.tenant_id=$1 AND i.plugin_id=$2")
                .bind(tenant).bind(dependency.plugin_id.as_str()).fetch_optional(&mut **tx).await.map_err(database_failure)?;
            let Some(row) = row else {
                if dependency.optional {
                    continue;
                }
                return Err(CommandFailure::precondition("DEPENDENCY_UNSATISFIED"));
            };
            if row
                .try_get::<String, _>("state")
                .map_err(database_failure)?
                != "ENABLED"
            {
                return Err(CommandFailure::precondition("DEPENDENCY_NOT_ENABLED"));
            }
            let version = Version::parse(
                &row.try_get::<String, _>("contract_version")
                    .map_err(database_failure)?,
            )
            .map_err(|_| checks::invalid())?;
            if !semver::VersionReq::parse(&dependency.contract_range)
                .map_err(|_| checks::invalid())?
                .matches(&version)
            {
                return Err(CommandFailure::precondition("DEPENDENCY_UNSATISFIED"));
            }
        }
        Ok(())
    }

    async fn has_active_plugin_runs(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        plugin: &str,
    ) -> Result<bool, CommandFailure> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE tenant_id=$1 AND plugin_id=$2 AND state NOT IN ('SUCCEEDED','FAILED','CANCELLED'))")
            .bind(tenant).bind(plugin).fetch_one(&mut **tx).await.map_err(database_failure)
    }

    async fn installation_projection(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let row = sqlx::query("SELECT plugin_id,artifact_digest,state,granted_capabilities,revocation_epoch,version FROM plugin_installations WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).fetch_one(&mut **tx).await.map_err(database_failure)?;
        let plugin: String = row.try_get("plugin_id").map_err(database_failure)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        Ok(Mutation::one(
            "PluginInstallation",
            id,
            version,
            json!({"installation_id":id,"plugin_id":plugin,"artifact_digest":row.try_get::<String,_>("artifact_digest").map_err(database_failure)?,
            "state":row.try_get::<String,_>("state").map_err(database_failure)?,"granted_capabilities":row.try_get::<Vec<String>,_>("granted_capabilities").map_err(database_failure)?,
            "revocation_epoch":row.try_get::<i64,_>("revocation_epoch").map_err(database_failure)?,"version":version}),
            plugin,
        ))
    }

    pub(crate) async fn quarantine_corrupt_artifact(
        &self,
        command: &AuthorizedCommand,
        reference: &ArtifactRef,
    ) -> Result<(), CommandFailure> {
        let mut tx = self.begin(&command.actor, true).await?;
        self.authorize_command(&mut tx, command).await?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        if reference.tenant_id != command.actor.tenant_id {
            return Err(CommandFailure::not_found());
        }
        // Carry the exact failed immutable reference across rollback. Looking up
        // the current plugin version here could quarantine the wrong package
        // after a concurrent upgrade and miss a tampered pinned version.
        let rows = sqlx::query("SELECT id,storage_key,digest,size_bytes FROM artifacts WHERE tenant_id=$1 AND id=$2 AND digest=$3 AND state='ACTIVE' AND size_bytes<=$4 FOR UPDATE")
            .bind(tenant).bind(parse_uuid(reference.artifact_id.as_str())?).bind(reference.digest.as_str()).bind(MAX_PACKAGE_BYTES as i64)
            .fetch_all(&mut *tx).await.map_err(database_failure)?;
        for row in rows {
            let key: String = row.try_get("storage_key").map_err(database_failure)?;
            let bytes = self.objects.get_bounded(&key, MAX_PACKAGE_BYTES).await?;
            let digest: String = row.try_get("digest").map_err(database_failure)?;
            let size: i64 = row.try_get("size_bytes").map_err(database_failure)?;
            if digest_bytes(&bytes).as_str() == digest && bytes.len() as i64 == size {
                continue;
            }
            let id: Uuid = row.try_get("id").map_err(database_failure)?;
            let version: i32 = sqlx::query_scalar("UPDATE artifacts SET state='QUARANTINED',version=version+1 WHERE tenant_id=$1 AND id=$2 RETURNING version")
                .bind(tenant).bind(id).fetch_one(&mut *tx).await.map_err(database_failure)?;
            let principal = parse_uuid(command.actor.principal_id.as_str())?;
            let correlation = Uuid::new_v4();
            sqlx::query("INSERT INTO audit_events(tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES($1,$2,$3,'artifact.quarantine','Artifact',$4,$5)")
                .bind(tenant).bind(Uuid::new_v4()).bind(principal).bind(id).bind(correlation).execute(&mut *tx).await.map_err(database_failure)?;
            sqlx::query("INSERT INTO outbox_events(tenant_id,id,aggregate_id,aggregate_version,event_name,aggregate_type,principal_id,correlation_id) VALUES($1,$2,$3,$4,'artifact.quarantine','Artifact',$5,$6)")
                .bind(tenant).bind(Uuid::new_v4()).bind(id).bind(version).bind(principal).bind(correlation).execute(&mut *tx).await.map_err(database_failure)?;
        }
        tx.commit().await.map_err(database_failure)
    }
}

fn manifest_projection(manifest: &PluginManifest) -> Result<ProjectionChange, CommandFailure> {
    Ok(ProjectionChange {
        resource: resource_ref("PluginManifest", Uuid::new_v4(), 1),
        value: serde_json::to_value(manifest).map_err(|_| checks::invalid())?,
        label: format!("{} {}", manifest.id, manifest.version),
        source_artifacts: Vec::new(),
    })
}

fn checked_version(command: &AuthorizedCommand, row: &PgRow) -> Result<i32, CommandFailure> {
    require_expected_version(
        row.try_get("version").map_err(database_failure)?,
        field(&command.input, "expected_version")?,
    )
}

fn require_reason(command: &AuthorizedCommand) -> Result<String, CommandFailure> {
    let reason: String = field(&command.input, "reason")?;
    if reason.trim().is_empty() {
        return Err(CommandFailure::invalid("REASON_REQUIRED"));
    }
    Ok(reason)
}

fn illegal() -> CommandFailure {
    CommandFailure::conflict("ILLEGAL_TRANSITION")
}

fn run_projection(row: &PgRow, tenant: Uuid) -> Result<ProjectionChange, CommandFailure> {
    let id: Uuid = row.try_get("id").map_err(database_failure)?;
    let version: i32 = row.try_get("version").map_err(database_failure)?;
    Ok(ProjectionChange {
        resource: resource_ref("Run", id, version),
        label: row.try_get("workflow_id").map_err(database_failure)?,
        source_artifacts: Vec::new(),
        value: json!({"run_id":id,"tenant_id":tenant,
        "workflow_id":row.try_get::<String,_>("workflow_id").map_err(database_failure)?,"workflow_version":row.try_get::<String,_>("workflow_version").map_err(database_failure)?,
        "state":row.try_get::<String,_>("state").map_err(database_failure)?,"plugin_digest":row.try_get::<String,_>("plugin_digest").map_err(database_failure)?,"grant_id":row.try_get::<Uuid,_>("grant_id").map_err(database_failure)?,
        "version":version,"created_at":row.try_get::<DateTime<Utc>,_>("created_at").map_err(database_failure)?,"updated_at":row.try_get::<DateTime<Utc>,_>("updated_at").map_err(database_failure)?}),
    })
}
