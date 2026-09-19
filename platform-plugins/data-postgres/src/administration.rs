use chrono::{DateTime, Utc};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, require_expected_version,
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::store::{Mutation, PostgresStore, Tx, database_failure, field, parse_uuid};

impl PostgresStore {
    pub(crate) async fn configure_budget(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let currency: String = field(&command.input, "currency")?;
        let period: String = field(&command.input, "period")?;
        let limit: i64 = field(&command.input, "limit_microunits")?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let id = Uuid::new_v4();
        // New budget dimensions begin at the default zero-limit version 1.
        // No provider call is reachable from configuration.
        sqlx::query("INSERT INTO budget_settings(tenant_id,id,currency,period,limit_microunits) VALUES($1,$2,$3,$4,0) ON CONFLICT(tenant_id,currency,period) DO NOTHING")
            .bind(tenant).bind(id).bind(&currency).bind(&period).execute(&mut **tx).await.map_err(database_failure)?;
        let current = sqlx::query("SELECT id,version FROM budget_settings WHERE tenant_id=$1 AND currency=$2 AND period=$3 FOR UPDATE")
            .bind(tenant).bind(&currency).bind(&period).fetch_one(&mut **tx).await.map_err(database_failure)?;
        let version = require_expected_version(
            current.try_get("version").map_err(database_failure)?,
            expected,
        )?;
        let id: Uuid = current.try_get("id").map_err(database_failure)?;
        let prefix = format!("{period}:{currency}:%");
        let outstanding: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_accounts WHERE tenant_id=$1 AND account_key LIKE $2 AND held_microunits+charged_microunits>$3)")
            .bind(tenant).bind(&prefix).bind(limit).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if outstanding {
            return Err(CommandFailure::precondition("BUDGET_BELOW_COMMITMENTS"));
        }
        sqlx::query("UPDATE budget_settings SET limit_microunits=$3,version=$4,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(id).bind(limit).bind(version).execute(&mut **tx).await.map_err(database_failure)?;
        sqlx::query("UPDATE budget_accounts SET limit_microunits=$3 WHERE tenant_id=$1 AND account_key LIKE $2")
            .bind(tenant).bind(prefix).bind(limit).execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "BudgetSettings",
            id,
            version,
            json!({"budget_id":id,"tenant_id":tenant,"currency":currency,"period":period,"limit_microunits":limit,"version":version}),
            format!("{period} {currency}"),
        ))
    }

    pub(crate) async fn set_kill_switch(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let scope: String = field(&command.input, "scope")?;
        let target: String = field(&command.input, "target_id")?;
        let active: bool = field(&command.input, "active")?;
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        let exists: bool = match scope.as_str() {
            "TENANT" => target==tenant.to_string(),
            "PLUGIN" => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2 AND state<>'UNINSTALLED')")
                .bind(tenant).bind(&target).fetch_one(&mut **tx).await.map_err(database_failure)?,
            "CONNECTION" => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM connections WHERE tenant_id=$1 AND id=$2)")
                .bind(tenant).bind(parse_uuid(&target)?).fetch_one(&mut **tx).await.map_err(database_failure)?,
            _ => false,
        };
        if !exists {
            return Err(CommandFailure::not_found());
        }
        let row = sqlx::query("INSERT INTO kill_switches(tenant_id,id,scope,target_id,active,reason,changed_by) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(tenant_id,scope,target_id) DO UPDATE SET active=EXCLUDED.active,reason=EXCLUDED.reason,changed_by=EXCLUDED.changed_by,version=kill_switches.version+1,updated_at=clock_timestamp() RETURNING id,version")
            .bind(tenant).bind(Uuid::new_v4()).bind(&scope).bind(&target).bind(active).bind(&reason).bind(principal)
            .fetch_one(&mut **tx).await.map_err(database_failure)?;
        let id: Uuid = row.try_get("id").map_err(database_failure)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        Ok(Mutation::one(
            "KillSwitch",
            id,
            version,
            json!({"switch_id":id,"scope":scope,"target_id":target,"active":active,"reason":reason,"version":version}),
            scope,
        ))
    }

    pub(crate) async fn require_dispatch_enabled(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        plugin: Option<&str>,
        connection: Option<Uuid>,
    ) -> Result<(), CommandFailure> {
        let stopped: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kill_switches WHERE tenant_id=$1 AND active AND ((scope='TENANT' AND target_id=$1::text) OR (scope='PLUGIN' AND target_id=$2) OR (scope='CONNECTION' AND target_id=$3)))")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(plugin).bind(connection.map(|id|id.to_string()))
            .fetch_one(&mut **tx).await.map_err(database_failure)?;
        if stopped {
            return Err(CommandFailure::precondition("KILL_SWITCH_ACTIVE"));
        }
        Ok(())
    }

    pub(crate) async fn mark_notification_read(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let id = parse_uuid(&field::<String>(&command.input, "notification_id")?)?;
        let row = sqlx::query("UPDATE notifications SET read_at=COALESCE(read_at,clock_timestamp()),version=version+1 WHERE tenant_id=$1 AND id=$2 AND recipient_id=$3 RETURNING resource_type,resource_id,subject_code,read_at,version")
            .bind(parse_uuid(command.actor.tenant_id.as_str())?).bind(id).bind(parse_uuid(command.actor.principal_id.as_str())?)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        Ok(Mutation::one(
            "Notification",
            id,
            version,
            json!({"notification_id":id,"resource_type":row.try_get::<String,_>("resource_type").map_err(database_failure)?,"resource_id":row.try_get::<String,_>("resource_id").map_err(database_failure)?,"subject_code":row.try_get::<String,_>("subject_code").map_err(database_failure)?,"read_at":row.try_get::<DateTime<Utc>,_>("read_at").map_err(database_failure)?,"version":version}),
            "Notification",
        ))
    }

    pub(crate) async fn request_support(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let actions: Vec<String> = field(&command.input, "actions")?;
        const METADATA_ACTIONS: &[&str] = &[
            "support.status.read",
            "support.health.read",
            "support.audit-metadata.read",
        ];
        if actions.is_empty()
            || actions
                .iter()
                .any(|a| !METADATA_ACTIONS.contains(&a.as_str()))
        {
            return Err(CommandFailure::denied("SUPPORT_SCOPE_DENIED"));
        }
        let expires: DateTime<Utc> = field(&command.input, "expires_at")?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        if expires <= now || expires > now + chrono::Duration::minutes(15) {
            return Err(CommandFailure::invalid("SUPPORT_EXPIRY_INVALID"));
        }
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO support_grants(tenant_id,id,requested_by,actions,reason,expires_at,state) VALUES($1,$2,$3,$4,$5,$6,'APPROVED')")
            .bind(parse_uuid(command.actor.tenant_id.as_str())?).bind(id).bind(parse_uuid(command.actor.principal_id.as_str())?)
            .bind(&actions).bind(&reason).bind(expires).execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "SupportGrant",
            id,
            1,
            json!({"support_grant_id":id,"actions":actions,"expires_at":expires,"state":"APPROVED","version":1}),
            "Metadata support access",
        ))
    }

    pub(crate) async fn configure_provider(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let profile: Value = field(&command.input, "profile")?;
        if profile["live_status"] != "DISABLED" {
            return Err(CommandFailure::precondition("LIVE_PROVIDER_DISABLED"));
        }
        let credential: String = field(&command.input, "credential_ref")?;
        let qualification: Option<Value> = field(&command.input, "qualification_ref")?;
        if qualification.is_some() {
            return Err(CommandFailure::precondition(
                "QUALIFICATION_REVIEW_REQUIRED",
            ));
        }
        let id = Uuid::new_v4();
        let profile_id = profile["id"]
            .as_str()
            .ok_or_else(|| CommandFailure::invalid("SCHEMA_INVALID"))?;
        sqlx::query("INSERT INTO provider_profiles(tenant_id,id,profile_id,profile,credential_ref) VALUES($1,$2,$3,$4,$5)")
            .bind(parse_uuid(command.actor.tenant_id.as_str())?).bind(id).bind(profile_id).bind(&profile).bind(credential)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "ProviderProfile",
            id,
            1,
            profile.clone(),
            profile_id,
        ))
    }

    pub(crate) async fn compact_provider(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let conversation_id: String = field(&command.input, "conversation_id")?;
        let profile_id: String = field(&command.input, "profile_id")?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let current = sqlx::query("SELECT id,version FROM provider_profiles WHERE tenant_id=$1 AND profile_id=$2 FOR UPDATE")
            .bind(tenant).bind(&profile_id).fetch_optional(&mut **tx).await.map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let version = require_expected_version(
            current.try_get("version").map_err(database_failure)?,
            expected,
        )?;
        let id: Uuid = current.try_get("id").map_err(database_failure)?;
        sqlx::query("UPDATE provider_profiles SET version=version+1 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        let new_version = version + 1;
        Ok(Mutation::one(
            "ProviderCompact",
            id,
            new_version,
            json!({"conversation_id":conversation_id,"profile_id":profile_id,"version":new_version}),
            format!("Compact {conversation_id}"),
        ))
    }

    pub(crate) async fn authorize_connection(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let provider: String = field(&command.input, "provider")?;
        let scopes: Vec<String> = field(&command.input, "requested_scopes")?;
        let return_to: String = field(&command.input, "return_to")?;
        let connection_id = Uuid::new_v4();
        // Seed connection row in PENDING if absent
        sqlx::query("INSERT INTO connections(tenant_id,id,provider,external_account,scopes,secret_ref,state,contract_version,capabilities) VALUES($1,$2,$3,$4,$5,'synthetic/secret','PENDING','1.0.0','{}') ON CONFLICT DO NOTHING")
            .bind(tenant).bind(connection_id).bind(&provider).bind(format!("synthetic/{connection_id}")).bind(&scopes)
            .execute(&mut **tx).await.map_err(database_failure)?;
        let auth_id = Uuid::new_v4();
        let state_digest = format!("sha256:{:064x}", auth_id.as_u128());
        let auth_url = format!("https://local-auth.example.test/authorize?state={auth_id}");
        let expires_at: DateTime<Utc> =
            sqlx::query_scalar("SELECT clock_timestamp()+interval '1 hour'")
                .fetch_one(&mut **tx)
                .await
                .map_err(database_failure)?;
        sqlx::query("INSERT INTO connector_authorizations(tenant_id,id,principal_id,connection_id,state_digest,authorization_url,return_to,secret_ref,requested_scopes,state,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'synthetic/secret',$8,'PENDING',$9)")
            .bind(tenant).bind(auth_id).bind(principal).bind(connection_id).bind(&state_digest).bind(&auth_url).bind(&return_to).bind(&scopes).bind(expires_at)
            .execute(&mut **tx).await.map_err(database_failure)?;
        // The projection value must satisfy the baseline ConnectorAuthorization
        // schema exactly: additionalProperties false, no version field, and the
        // required tenant_id/expires_at members present.
        Ok(Mutation::one(
            "ConnectorAuthorization",
            auth_id,
            1,
            json!({"authorization_id":auth_id,"tenant_id":tenant,"connection_id":connection_id,"authorization_url":auth_url,"return_to":return_to,"expires_at":expires_at,"state":"PENDING"}),
            format!("Authorize {provider}"),
        ))
    }

    pub(crate) async fn create_schedule(
        &self,
        _tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let workflow_id: String = field(&command.input, "workflow_id")?;
        let cron: String = field(&command.input, "cron")?;
        let timezone: String = field(&command.input, "timezone")?;
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "Schedule",
            id,
            1,
            json!({"schedule_id":id,"workflow_id":workflow_id,"cron":cron,"timezone":timezone,"version":1}),
            format!("Schedule {workflow_id}"),
        ))
    }

    pub(crate) async fn replay_deadletter(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let event_id = parse_uuid(&field::<String>(&command.input, "event_id")?)?;
        let reason: String = field(&command.input, "reason")?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let updated = sqlx::query("UPDATE outbox_events SET delivery_status='PENDING',lease_expires_at=NULL WHERE tenant_id=$1 AND id=$2 AND delivery_status='DEAD_LETTER'")
            .bind(tenant).bind(event_id).execute(&mut **tx).await.map_err(database_failure)?;
        if updated.rows_affected() == 0 {
            return Err(CommandFailure::not_found());
        }
        Ok(Mutation::one(
            "DeadletterReplay",
            event_id,
            1,
            json!({"event_id":event_id,"reason":reason,"version":1}),
            format!("Replay {event_id}"),
        ))
    }

    pub(crate) async fn register_ui(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let plugin_id: String = field(&command.input, "plugin_id")?;
        let contributions: Vec<Value> = field(&command.input, "contributions")?;
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        for c in &contributions {
            let cid = c["id"].as_str().unwrap_or("default");
            let path = c["path"].as_str().unwrap_or("/");
            let slot = c["slot"].as_str().unwrap_or("NAVIGATION");
            // REQ-122: a route/slot already claimed by another plugin (or by a
            // different contribution id of the same plugin) must reject the
            // registration, never silently overwrite it. Same-plugin same-id
            // re-registration is the idempotent replay path and rewrites in
            // place.
            let written = sqlx::query("INSERT INTO ui_contributions(tenant_id,plugin_id,contribution_id,contribution,path,slot) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(tenant_id,path,slot) DO UPDATE SET contribution=EXCLUDED.contribution WHERE ui_contributions.plugin_id=$2 AND ui_contributions.contribution_id=$3")
                .bind(tenant).bind(&plugin_id).bind(cid).bind(c).bind(path).bind(slot)
                .execute(&mut **tx).await.map_err(database_failure)?;
            if written.rows_affected() == 0 {
                return Err(CommandFailure::conflict("UI_CONTRIBUTION_CONFLICT"));
            }
        }
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "UIRegistration",
            id,
            1,
            json!({"plugin_id":plugin_id,"contributions_count":contributions.len(),"version":1}),
            format!("UI {plugin_id}"),
        ))
    }

    pub(crate) async fn create_export(
        &self,
        _tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let resource_type: String = field(&command.input, "resource_type")?;
        let format: String = field(&command.input, "format")?;
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "Export",
            id,
            1,
            json!({"export_id":id,"resource_type":resource_type,"format":format,"state":"PENDING","version":1}),
            format!("Export {resource_type}"),
        ))
    }

    pub(crate) async fn revoke_connection(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let connection_id = parse_uuid(&field::<String>(&command.input, "connection_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        let current = sqlx::query("SELECT provider,external_account,scopes,secret_ref,contract_version,capabilities,verified_at,version FROM connections WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(connection_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let current_version: i32 = current.try_get("version").map_err(database_failure)?;
        let version = require_expected_version(current_version, expected)?;
        let new_version = version + 1;
        sqlx::query("UPDATE connections SET state='REVOKED',version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(connection_id)
            .bind(new_version)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        Ok(Mutation::one(
            "Connection",
            connection_id,
            new_version,
            json!({
                "connection_id": connection_id.to_string(),
                "tenant_id": tenant.to_string(),
                "provider": current.try_get::<String, _>("provider").map_err(database_failure)?,
                "external_account": current.try_get::<String, _>("external_account").map_err(database_failure)?,
                "scopes": current.try_get::<Vec<String>, _>("scopes").map_err(database_failure)?,
                "secret_ref": current.try_get::<String, _>("secret_ref").map_err(database_failure)?,
                "state": "REVOKED",
                "contract_version": current.try_get::<String, _>("contract_version").map_err(database_failure)?,
                "capabilities": current.try_get::<Vec<String>, _>("capabilities").map_err(database_failure)?,
                "verified_at": current.try_get::<Option<DateTime<Utc>>, _>("verified_at").map_err(database_failure)?,
                "version": new_version
            }),
            format!("Revoke {connection_id}"),
        ))
    }

    pub(crate) async fn qualify_release(
        &self,
        _tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let artifact_digest: String = field(&command.input, "artifact_digest")?;
        let evidence_ref: Value = field(&command.input, "evidence_ref")?;
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "ReleaseQualification",
            id,
            1,
            json!({"qualification_id":id,"artifact_digest":artifact_digest,"evidence_ref":evidence_ref,"state":"QUALIFIED","version":1}),
            format!("Qualify {artifact_digest}"),
        ))
    }

    pub(crate) async fn submit_catalog(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let manifest: Value = field(&command.input, "manifest")?;
        let support_contact: String = field(&command.input, "support_contact")?;
        let description: String = field(&command.input, "description")?;
        let publisher_id = manifest["publisher_id"]
            .as_str()
            .ok_or_else(|| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let plugin_id = manifest["id"]
            .as_str()
            .ok_or_else(|| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let artifact_digest = manifest["artifact_digest"]
            .as_str()
            .ok_or_else(|| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO catalog_listings(tenant_id,id,publisher_id,plugin_id,artifact_digest,manifest,support_contact,description,submitted_by,state,version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'PENDING',1)")
            .bind(tenant)
            .bind(id)
            .bind(publisher_id)
            .bind(plugin_id)
            .bind(artifact_digest)
            .bind(&manifest)
            .bind(&support_contact)
            .bind(&description)
            .bind(parse_uuid(command.actor.principal_id.as_str())?)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        Ok(Mutation::one(
            "CatalogListing",
            id,
            1,
            json!({"listing_id":id,"tenant_id":tenant,"publisher_id":publisher_id,"plugin_id":plugin_id,"artifact_digest":artifact_digest,"manifest":manifest,"support_contact":support_contact,"description":description,"submitted_by":command.actor.principal_id,"state":"PENDING","version":1}),
            format!("Submit {plugin_id}"),
        ))
    }

    pub(crate) async fn review_catalog(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let listing_id = parse_uuid(&field::<String>(&command.input, "listing_id")?)?;
        let decision: String = field(&command.input, "decision")?;
        let evidence_ref: Value = field(&command.input, "evidence_ref")?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let current = sqlx::query("SELECT plugin_id,state,version FROM catalog_listings WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(listing_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let current_version: i32 = current.try_get("version").map_err(database_failure)?;
        let version = require_expected_version(current_version, expected)?;
        let state = match decision.as_str() {
            "APPROVE" => "APPROVED",
            "REJECT" => "REJECTED",
            _ => return Err(CommandFailure::invalid("SCHEMA_INVALID")),
        };
        let new_version = version + 1;
        sqlx::query("UPDATE catalog_listings SET state=$3,reviewed_by=$4,evidence_ref=$5,version=$6 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(listing_id)
            .bind(state)
            .bind(parse_uuid(command.actor.principal_id.as_str())?)
            .bind(&evidence_ref)
            .bind(new_version)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        Ok(Mutation::one(
            "CatalogListing",
            listing_id,
            new_version,
            json!({"listing_id":listing_id,"tenant_id":tenant,"plugin_id":current.try_get::<String,_>("plugin_id").map_err(database_failure)?,"state":state,"reviewed_by":command.actor.principal_id,"evidence_ref":evidence_ref,"version":new_version}),
            format!("Review {listing_id}"),
        ))
    }

    pub(crate) async fn run_conformance(
        &self,
        _tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let artifact_digest: String = field(&command.input, "artifact_digest")?;
        let suite_version: String = field(&command.input, "suite_version")?;
        let profile: String = field(&command.input, "profile")?;
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "ConformanceRun",
            id,
            1,
            json!({"run_id":id,"artifact_digest":artifact_digest,"suite_version":suite_version,"profile":profile,"state":"COMPLETED","version":1}),
            format!("Conformance {suite_version}"),
        ))
    }
}
