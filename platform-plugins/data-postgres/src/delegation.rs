use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use masonwing_contracts::wire::{PrincipalKind, PrincipalRef, ResourceRef};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, require_expected_version,
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::store::{
    Mutation, PostgresStore, ProjectionChange, Tx, database_failure, field, parse_uuid,
    resource_ref,
};

impl PostgresStore {
    pub(crate) async fn create_grant(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let delegate: PrincipalRef = field(&command.input, "delegate")?;
        let mut actions: Vec<String> = field(&command.input, "actions")?;
        let resources: Vec<ResourceRef> = field(&command.input, "resources")?;
        let expires: DateTime<Utc> = field(&command.input, "expires_at")?;
        let parent: Option<String> = field(&command.input, "parent_grant_id")?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        if expires <= now {
            return Err(CommandFailure::invalid("GRANT_EXPIRED"));
        }
        if actions.is_empty() || resources.is_empty() {
            return Err(CommandFailure::denied("GRANT_SCOPE_EMPTY"));
        }
        if delegate.kind == PrincipalKind::Support {
            return Err(CommandFailure::denied("SUPPORT_SCOPE_DENIED"));
        }
        actions.sort();
        actions.dedup();
        let resource_set: BTreeSet<_> = resources
            .iter()
            .map(|r| (&r.resource_type, r.resource_id.as_str(), r.version))
            .collect();
        if resource_set.len() != resources.len() {
            return Err(CommandFailure::invalid("GRANT_DUPLICATE_RESOURCE"));
        }
        let policy = self.read_policy(tx, &command.actor).await?;
        // Every grant scope must refer to an existing current host resource. Each
        // delegated action is evaluated as the creator against that resource.
        for resource in &resources {
            let current = if resource.resource_type == "Artifact" {
                // Raw input artifacts are first-class grant resources. Their
                // identity/version comes from active metadata, not a fabricated
                // generic projection or an ID supplied by the caller.
                self.artifact_metadata_in(tx, &command.actor, resource.resource_id.as_str())
                    .await?;
                sqlx::query("SELECT version,classification FROM artifacts WHERE tenant_id=$1 AND id=$2 AND state='ACTIVE'")
                    .bind(tenant).bind(parse_uuid(resource.resource_id.as_str())?)
                    .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?
            } else {
                sqlx::query("SELECT p.version,a.classification FROM resource_projections p JOIN artifacts a ON a.tenant_id=p.tenant_id AND a.id=p.artifact_id WHERE p.tenant_id=$1 AND p.resource_type=$2 AND p.resource_id=$3 AND a.state='ACTIVE' ORDER BY p.version DESC LIMIT 1")
                    .bind(tenant).bind(&resource.resource_type).bind(resource.resource_id.as_str())
                    .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?
            };
            if current
                .try_get::<i32, _>("version")
                .map_err(database_failure)?
                != resource.version
            {
                return Err(CommandFailure::conflict("STALE_VERSION"));
            }
            for action in &actions {
                if !policy.permits(
                    action,
                    &resource.resource_type,
                    resource.resource_id.as_str(),
                    BTreeMap::from([
                        ("version".into(), json!(resource.version)),
                        (
                            "classification".into(),
                            json!(
                                current
                                    .try_get::<String, _>("classification")
                                    .map_err(database_failure)?
                            ),
                        ),
                    ]),
                ) {
                    return Err(CommandFailure::denied("GRANT_SCOPE_DENIED"));
                }
            }
        }
        if let Some(parent_id) = &parent {
            let row = self
                .require_active_grant(tx, &command.actor, parse_uuid(parent_id)?, None, None)
                .await?;
            if row.expires_at < expires
                || actions.iter().any(|a| !row.actions.contains(a))
                || resources.iter().any(|r| !row.resources.contains(r))
            {
                return Err(CommandFailure::denied("GRANT_SCOPE_DENIED"));
            }
        }
        let id = Uuid::new_v4();
        let kind = match delegate.kind {
            PrincipalKind::User => "USER",
            PrincipalKind::Service => "SERVICE",
            PrincipalKind::Automation => "AUTOMATION",
            PrincipalKind::Support => "SUPPORT",
        };
        sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,parent_grant_id,expires_at,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'ACTIVE')")
            .bind(tenant).bind(id).bind(delegate.id.as_str()).bind(kind).bind(&delegate.issuer).bind(parse_uuid(command.actor.principal_id.as_str())?)
            .bind(&actions).bind(json!(resources)).bind(parent.as_deref().map(parse_uuid).transpose()?).bind(expires)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "Delegation",
            id,
            1,
            json!({"grant_id":id,"tenant_id":tenant,"principal":delegate,"actions":actions,"resources":resources,"expires_at":expires,"parent_grant_id":parent,"state":"ACTIVE","version":1}),
            format!("Grant to {}", delegate.id),
        ))
    }

    pub(crate) async fn revoke_grant(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let id = parse_uuid(&field::<String>(&command.input, "grant_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        let row = sqlx::query(
            "SELECT version,state FROM delegations WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(tenant)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        let version =
            require_expected_version(row.try_get("version").map_err(database_failure)?, expected)?;
        if row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            != "ACTIVE"
        {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let rows=sqlx::query("WITH RECURSIVE descendants AS (SELECT id FROM delegations WHERE tenant_id=$1 AND id=$2 UNION ALL SELECT d.id FROM delegations d JOIN descendants p ON d.parent_grant_id=p.id WHERE d.tenant_id=$1) UPDATE delegations d SET state='REVOKED',version=version+1,fence=fence+1 FROM descendants x WHERE d.tenant_id=$1 AND d.id=x.id AND d.state='ACTIVE' RETURNING d.*")
            .bind(tenant).bind(id).fetch_all(&mut **tx).await.map_err(database_failure)?;
        let mut changes = Vec::new();
        for row in rows {
            let changed_id: Uuid = row.try_get("id").map_err(database_failure)?;
            let value = grant_json(&row)?;
            changes.push(ProjectionChange {
                resource: resource_ref(
                    "Delegation",
                    changed_id,
                    row.try_get("version").map_err(database_failure)?,
                ),
                value,
                label: "Revoked delegation".into(),
                source_artifacts: Vec::new(),
            });
        }
        // Fence every descendant immediately. The workflow/effect admission path
        // rechecks this stored fence; reconciliation of already-sent calls stays
        // available, so revocation cannot falsely report a remote action undone.
        Ok(Mutation {
            primary: resource_ref("Delegation", id, version),
            changes,
            run_id: None,
            effect_id: None,
            accepted: false,
        })
    }

    pub(crate) async fn require_active_grant(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        id: Uuid,
        action: Option<&str>,
        resource: Option<&ResourceRef>,
    ) -> Result<ActiveGrant, CommandFailure> {
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let rows=sqlx::query("WITH RECURSIVE ancestry AS (SELECT d.*,ARRAY[d.id] AS path FROM delegations d WHERE d.tenant_id=$1 AND d.id=$2 UNION ALL SELECT d.*,a.path||d.id FROM delegations d JOIN ancestry a ON a.parent_grant_id=d.id WHERE d.tenant_id=$1 AND NOT d.id=ANY(a.path)) SELECT *,clock_timestamp() AS checked_at FROM ancestry")
            .bind(tenant).bind(id).fetch_all(&mut **tx).await.map_err(database_failure)?;
        if rows.is_empty() {
            return Err(CommandFailure::not_found());
        }
        for row in &rows {
            if row
                .try_get::<String, _>("state")
                .map_err(database_failure)?
                != "ACTIVE"
                || row
                    .try_get::<DateTime<Utc>, _>("expires_at")
                    .map_err(database_failure)?
                    <= row
                        .try_get::<DateTime<Utc>, _>("checked_at")
                        .map_err(database_failure)?
            {
                return Err(CommandFailure::denied("GRANT_REVOKED_OR_EXPIRED"));
            }
        }
        let row = rows
            .iter()
            .find(|r| r.get::<Uuid, _>("id") == id)
            .ok_or_else(CommandFailure::not_found)?;
        let creator: Uuid = row.try_get("creator_id").map_err(database_failure)?;
        let delegate: String = row.try_get("principal_id").map_err(database_failure)?;
        let issuer: String = row.try_get("issuer").map_err(database_failure)?;
        if creator.to_string() != actor.principal_id.as_str()
            && (delegate != actor.principal_id.as_str() || issuer != actor.issuer)
        {
            return Err(CommandFailure::denied("GRANT_PRINCIPAL_MISMATCH"));
        }
        let actions: Vec<String> = row.try_get("actions").map_err(database_failure)?;
        let resources: Vec<ResourceRef> = serde_json::from_value(
            row.try_get::<Value, _>("resources")
                .map_err(database_failure)?,
        )
        .map_err(|_| CommandFailure::unavailable("GRANT_INTEGRITY"))?;
        if action.is_some_and(|a| !actions.iter().any(|v| v == a))
            || resource.is_some_and(|r| !resources.contains(r))
        {
            return Err(CommandFailure::denied("CAPABILITY_DENIED"));
        }
        Ok(ActiveGrant {
            id,
            actions,
            resources,
            expires_at: row.try_get("expires_at").map_err(database_failure)?,
            fence: row.try_get("fence").map_err(database_failure)?,
        })
    }
}

pub(crate) struct ActiveGrant {
    pub id: Uuid,
    pub actions: Vec<String>,
    pub resources: Vec<ResourceRef>,
    pub expires_at: DateTime<Utc>,
    pub fence: i64,
}

fn grant_json(row: &sqlx::postgres::PgRow) -> Result<Value, CommandFailure> {
    Ok(
        json!({"grant_id":row.try_get::<Uuid,_>("id").map_err(database_failure)?,"tenant_id":row.try_get::<Uuid,_>("tenant_id").map_err(database_failure)?,"principal":{"type":row.try_get::<String,_>("principal_type").map_err(database_failure)?,"id":row.try_get::<String,_>("principal_id").map_err(database_failure)?,"issuer":row.try_get::<String,_>("issuer").map_err(database_failure)?},"actions":row.try_get::<Vec<String>,_>("actions").map_err(database_failure)?,"resources":row.try_get::<Value,_>("resources").map_err(database_failure)?,"expires_at":row.try_get::<DateTime<Utc>,_>("expires_at").map_err(database_failure)?,"parent_grant_id":row.try_get::<Option<Uuid>,_>("parent_grant_id").map_err(database_failure)?,"state":row.try_get::<String,_>("state").map_err(database_failure)?,"version":row.try_get::<i32,_>("version").map_err(database_failure)?}),
    )
}
