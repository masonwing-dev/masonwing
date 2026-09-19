//! Resource facts for command authorization come from the host's current rows.
//! Client-supplied tenant, role or Cedar entities cannot become authority.

use crate::store::{PostgresStore, Tx, database_failure, field, parse_uuid};
use masonwing_kernel::runtime::{AuthorizedCommand, CommandFailure};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::BTreeMap;

impl PostgresStore {
    pub(crate) async fn authorize_command(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<(), CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let policy = self.read_policy(tx, &command.actor).await?;
        let (kind, id, attributes) = match command.operation.as_str() {
            "plugin.enable" | "plugin.disable" | "plugin.upgrade" | "plugin.revoke"
            | "plugin.uninstall" | "plugin.invoke" => {
                let plugin: String = field(&command.input, "plugin_id")?;
                let row=sqlx::query("SELECT id,version,state FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2")
                    .bind(tenant).bind(&plugin).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
                (
                    "PluginInstallation".to_owned(),
                    row.try_get::<uuid::Uuid, _>("id")
                        .map_err(database_failure)?
                        .to_string(),
                    BTreeMap::from([
                        ("plugin_id".into(), json!(plugin)),
                        (
                            "version".into(),
                            json!(row.try_get::<i32, _>("version").map_err(database_failure)?),
                        ),
                        (
                            "state".into(),
                            json!(
                                row.try_get::<String, _>("state")
                                    .map_err(database_failure)?
                            ),
                        ),
                    ]),
                )
            }
            "membership.change" | "membership.revoke" => {
                let id: String = field(&command.input, "membership_id")?;
                let row=sqlx::query("SELECT version,status,principal_id FROM memberships WHERE tenant_id=$1 AND membership_id=$2")
                    .bind(tenant).bind(parse_uuid(&id)?).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
                (
                    "Membership".into(),
                    id,
                    BTreeMap::from([
                        (
                            "version".into(),
                            json!(row.try_get::<i32, _>("version").map_err(database_failure)?),
                        ),
                        (
                            "state".into(),
                            json!(
                                row.try_get::<String, _>("status")
                                    .map_err(database_failure)?
                            ),
                        ),
                        (
                            "principal_id".into(),
                            json!(
                                row.try_get::<uuid::Uuid, _>("principal_id")
                                    .map_err(database_failure)?
                                    .to_string()
                            ),
                        ),
                    ]),
                )
            }
            "artifact.finalize" => {
                let id: String = field(&command.input, "artifact_id")?;
                let row=sqlx::query("SELECT version,state,classification,creator_id FROM artifacts WHERE tenant_id=$1 AND id=$2 AND state<>'PURGED'")
                    .bind(tenant).bind(parse_uuid(&id)?).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
                (
                    "Artifact".into(),
                    id,
                    BTreeMap::from([
                        (
                            "version".into(),
                            json!(row.try_get::<i32, _>("version").map_err(database_failure)?),
                        ),
                        (
                            "classification".into(),
                            json!(
                                row.try_get::<String, _>("classification")
                                    .map_err(database_failure)?
                            ),
                        ),
                        (
                            "creator_id".into(),
                            json!(
                                row.try_get::<uuid::Uuid, _>("creator_id")
                                    .map_err(database_failure)?
                                    .to_string()
                            ),
                        ),
                    ]),
                )
            }
            operation => {
                let target = match operation {
                    "grant.revoke" => Some(("Delegation", "grant_id")),
                    "run.cancel" => Some(("Run", "run_id")),
                    "approval.decide" => Some(("Approval", "proposal_id")),
                    "effect.dispatch" | "effect.reconcile" | "effect.compensate" => {
                        Some(("EffectIntent", "effect_id"))
                    }
                    "connection.revoke" => Some(("Connection", "connection_id")),
                    "notification.read" => Some(("Notification", "notification_id")),
                    "catalog.review" => Some(("CatalogListing", "listing_id")),
                    _ => None,
                };
                if let Some((kind, field_name)) = target {
                    let id: String = field(&command.input, field_name)?;
                    let row=sqlx::query("SELECT p.version,a.classification FROM resource_projections p JOIN artifacts a ON a.tenant_id=p.tenant_id AND a.id=p.artifact_id WHERE p.tenant_id=$1 AND p.resource_type=$2 AND p.resource_id=$3 AND a.state='ACTIVE' ORDER BY p.version DESC LIMIT 1")
                        .bind(tenant).bind(kind).bind(&id).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
                    (
                        kind.into(),
                        id,
                        BTreeMap::from([
                            (
                                "version".into(),
                                json!(row.try_get::<i32, _>("version").map_err(database_failure)?),
                            ),
                            (
                                "classification".into(),
                                json!(
                                    row.try_get::<String, _>("classification")
                                        .map_err(database_failure)?
                                ),
                            ),
                        ]),
                    )
                } else {
                    (
                        "Tenant".into(),
                        tenant.to_string(),
                        BTreeMap::<String, Value>::new(),
                    )
                }
            }
        };
        if !policy.permits(&command.operation, &kind, &id, attributes) {
            return Err(CommandFailure::denied("AUTHZ_DENIED"));
        }
        Ok(())
    }
}
