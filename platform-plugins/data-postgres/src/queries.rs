use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use masonwing_authorization_cedar::{
    CedarAuthorizationAdapter, CedarPolicySnapshot, TrustedAuthorizationRequest, TrustedPrincipal,
    TrustedResource,
};
use masonwing_contract_validation::{canonical_bytes, digest_bytes};
use masonwing_contracts::{
    ArtifactId, Digest,
    wire::{ArtifactRef, Classification, ReadCollection, ResourceProjection},
};
use masonwing_kernel::runtime::{
    CommandActor, CommandFailure, FailureKind, PageQuery, StoredArtifact,
};
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use crate::store::{PostgresStore, Tx, database_failure, parse_uuid, resource_ref};

pub const BASELINE_RESOURCE_TYPES: &[&str] = &[
    "Run",
    "Membership",
    "Delegation",
    "EffectIntent",
    "Connection",
    "CostReservation",
    "PluginManifest",
    "UploadSession",
    "ConnectorAuthorization",
];
/// Host extensions have their own route, keeping the baseline's closed enum intact.
pub const EXTENSION_RESOURCE_TYPES: &[&str] = &[
    "ProductComposition",
    "PluginInstallation",
    "PluginInvocation",
    "Approval",
    "Notification",
    "BudgetSettings",
    "KillSwitch",
    "ProviderProfile",
    "SupportGrant",
    "CatalogListing",
    "Export",
    "ReleaseQualification",
    "ConformanceRun",
    "PolicyProposal",
    "Schedule",
    "DeletionRequest",
    "PolicyDecision",
    "MembershipInvitation",
    "IdentityConfigurationProposal",
];

pub(crate) struct ReadPolicy {
    snapshot: CedarPolicySnapshot,
    principal: TrustedPrincipal,
}

impl ReadPolicy {
    pub(crate) fn permits(
        &self,
        action: &str,
        kind: &str,
        id: &str,
        attributes: BTreeMap<String, Value>,
    ) -> bool {
        CedarAuthorizationAdapter
            .evaluate(
                &self.snapshot,
                &TrustedAuthorizationRequest {
                    principal: self.principal.clone(),
                    action: action.to_owned(),
                    resource: TrustedResource {
                        entity_type: kind.to_owned(),
                        resource_id: id.to_owned(),
                        tenant_id: self.principal.tenant_id.clone(),
                        attributes,
                    },
                    context: BTreeMap::new(),
                },
            )
            .is_ok()
    }
}

impl PostgresStore {
    pub(crate) async fn read_policy(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
    ) -> Result<ReadPolicy, CommandFailure> {
        let row = sqlx::query("SELECT m.role,ARRAY(SELECT mr.role FROM membership_roles mr WHERE mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id ORDER BY mr.role) AS roles,p.cedar_source,p.policy_version,p.policy_epoch FROM memberships m JOIN authorization_policies p ON p.tenant_id=m.tenant_id AND p.is_current WHERE m.tenant_id=$1 AND m.principal_id=$2 AND m.status='ACTIVE'")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(parse_uuid(actor.principal_id.as_str())?)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let snapshot = CedarPolicySnapshot::parse(
            row.try_get::<String, _>("policy_version")
                .map_err(database_failure)?,
            row.try_get::<i64, _>("policy_epoch")
                .map_err(database_failure)?,
            &row.try_get::<String, _>("cedar_source")
                .map_err(database_failure)?,
        )
        .map_err(|_| CommandFailure::unavailable("POLICY_INVALID"))?;
        Ok(ReadPolicy {
            snapshot,
            principal: TrustedPrincipal {
                principal_id: actor.principal_id.to_string(),
                tenant_id: actor.tenant_id.to_string(),
                role: row.try_get("role").map_err(database_failure)?,
                membership_epoch: actor.membership_epoch,
                roles: row.try_get("roles").map_err(database_failure)?,
                permission_epoch: actor.permission_epoch,
            },
        })
    }

    /// Read the current active policy snapshot for any principal in the tenant
    /// (used by the worker relay during pre-dispatch authority re-check).
    pub(crate) async fn read_policy_for_principal(
        &self,
        tx: &mut Tx<'_>,
        tenant_id: uuid::Uuid,
        principal_id: uuid::Uuid,
    ) -> Result<ReadPolicy, CommandFailure> {
        let row = sqlx::query("SELECT m.role,m.membership_epoch,m.permission_epoch,ARRAY(SELECT mr.role FROM membership_roles mr WHERE mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id ORDER BY mr.role) AS roles,p.cedar_source,p.policy_version,p.policy_epoch FROM memberships m JOIN authorization_policies p ON p.tenant_id=m.tenant_id AND p.is_current WHERE m.tenant_id=$1 AND m.principal_id=$2 AND m.status='ACTIVE'")
            .bind(tenant_id).bind(principal_id)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let snapshot = CedarPolicySnapshot::parse(
            row.try_get::<String, _>("policy_version")
                .map_err(database_failure)?,
            row.try_get::<i64, _>("policy_epoch")
                .map_err(database_failure)?,
            &row.try_get::<String, _>("cedar_source")
                .map_err(database_failure)?,
        )
        .map_err(|_| CommandFailure::unavailable("POLICY_INVALID"))?;
        Ok(ReadPolicy {
            snapshot,
            principal: TrustedPrincipal {
                principal_id: principal_id.to_string(),
                tenant_id: tenant_id.to_string(),
                role: row.try_get("role").map_err(database_failure)?,
                membership_epoch: row.try_get("membership_epoch").map_err(database_failure)?,
                roles: row.try_get("roles").map_err(database_failure)?,
                permission_epoch: row.try_get("permission_epoch").map_err(database_failure)?,
            },
        })
    }

    pub async fn read_collection(
        &self,
        actor: &CommandActor,
        kind: &str,
        page: &PageQuery,
    ) -> Result<ReadCollection, CommandFailure> {
        if !BASELINE_RESOURCE_TYPES.contains(&kind) && !EXTENSION_RESOURCE_TYPES.contains(&kind) {
            return Err(CommandFailure::not_found());
        }
        let limit = page.checked_limit()?;
        let query = page.q.as_deref().unwrap_or("").trim();
        if query.len() > 256 {
            return Err(CommandFailure::invalid("SEARCH_QUERY_INVALID"));
        }
        let query_digest = digest_bytes(
            &canonical_bytes(&json!({"q":query,"type":kind,"limit":limit}))
                .map_err(|_| CommandFailure::invalid("SEARCH_QUERY_INVALID"))?,
        );
        let mut tx = self.begin(actor, false).await?;
        let policy = self.read_policy(&mut tx, actor).await?;
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let principal = parse_uuid(actor.principal_id.as_str())?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
        let (snapshot_at, snapshot_sequence, mut last_at, mut last_id) = if let Some(cursor) =
            &page.cursor
        {
            let id = Uuid::parse_str(cursor).map_err(|_| snapshot_required())?;
            let row = sqlx::query("SELECT snapshot_at,snapshot_sequence,last_created_at,last_resource_id FROM read_cursors WHERE tenant_id=$1 AND id=$2 AND principal_id=$3 AND membership_epoch=$4 AND permission_epoch=$5 AND policy_version=$6 AND policy_epoch=$7 AND resource_type=$8 AND query_digest=$9 AND page_limit=$10 AND expires_at>$11")
                .bind(tenant).bind(id).bind(principal).bind(actor.membership_epoch).bind(actor.permission_epoch)
                .bind(&actor.policy_version).bind(actor.policy_epoch).bind(kind).bind(query_digest.as_str())
                .bind(limit as i32).bind(now).fetch_optional(&mut *tx).await.map_err(database_failure)?.ok_or_else(snapshot_required)?;
            (
                row.try_get("snapshot_at").map_err(database_failure)?,
                row.try_get("snapshot_sequence").map_err(database_failure)?,
                Some(
                    row.try_get::<DateTime<Utc>, _>("last_created_at")
                        .map_err(database_failure)?,
                ),
                Some(
                    row.try_get::<String, _>("last_resource_id")
                        .map_err(database_failure)?,
                ),
            )
        } else {
            let sequence: i64 = sqlx::query_scalar("SELECT COALESCE(max(revision_sequence),0) FROM resource_projections WHERE tenant_id=$1")
                .bind(tenant).fetch_one(&mut *tx).await.map_err(database_failure)?;
            (now, sequence, None, None)
        };
        let mut items = Vec::new();
        let mut more = false;
        let mut scanned = 0;
        // Bound every request even for a policy which denies all candidates. A
        // continuation resumes after the last examined tuple, so it cannot leak
        // a count of hidden rows or lose visible entries behind filtered rows.
        while items.len() < limit as usize && scanned < 2000 {
            let rows = sqlx::query(
                "WITH latest AS (SELECT DISTINCT ON (p.resource_id) p.* FROM resource_projections p WHERE p.tenant_id=$1 AND p.resource_type=$2 AND p.revision_sequence<=$3 ORDER BY p.resource_id,p.version DESC) SELECT p.resource_id,p.version,p.created_at,a.classification FROM latest p JOIN artifacts a ON a.tenant_id=p.tenant_id AND a.id=p.artifact_id WHERE a.state='ACTIVE' AND ($4::timestamptz IS NULL OR (p.created_at,p.resource_id)>($4,$5)) AND ($6='' OR p.search_label ILIKE $6 ESCAPE '\\') AND NOT EXISTS(SELECT 1 FROM deletion_tombstones d WHERE d.tenant_id=p.tenant_id AND d.resource_type=p.resource_type AND d.resource_id=p.resource_id) ORDER BY p.created_at,p.resource_id LIMIT 201")
                .bind(tenant).bind(kind).bind(snapshot_sequence).bind(last_at).bind(last_id.as_deref())
                .bind(if query.is_empty() { String::new() } else { format!("%{}%", escape_like(query)) })
                .fetch_all(&mut *tx).await.map_err(database_failure)?;
            if rows.is_empty() {
                more = false;
                break;
            }
            let batch_has_more = rows.len() > 200;
            let batch_length = rows.len().min(200);
            for (index, row) in rows.iter().take(200).enumerate() {
                scanned += 1;
                let id: String = row.try_get("resource_id").map_err(database_failure)?;
                let version: i32 = row.try_get("version").map_err(database_failure)?;
                last_at = Some(row.try_get("created_at").map_err(database_failure)?);
                last_id = Some(id.clone());
                let attrs = BTreeMap::from([
                    (
                        "classification".into(),
                        json!(
                            row.try_get::<String, _>("classification")
                                .map_err(database_failure)?
                        ),
                    ),
                    ("version".into(), json!(version)),
                ]);
                if policy.permits("resource.read", kind, &id, attrs) {
                    items.push(resource_ref(kind, parse_uuid(&id)?, version));
                }
                more = batch_has_more || index + 1 < batch_length;
                if items.len() == limit as usize {
                    break;
                }
            }
            if !more {
                break;
            }
        }
        let next_cursor = if more {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO read_cursors(tenant_id,id,principal_id,membership_epoch,permission_epoch,policy_version,policy_epoch,resource_type,query_digest,snapshot_at,snapshot_sequence,last_created_at,last_resource_id,page_limit,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$10+interval '15 minutes')")
                .bind(tenant).bind(id).bind(principal).bind(actor.membership_epoch).bind(actor.permission_epoch)
                .bind(&actor.policy_version).bind(actor.policy_epoch).bind(kind).bind(query_digest.as_str())
                .bind(snapshot_at).bind(snapshot_sequence).bind(last_at).bind(last_id).bind(limit as i32)
                .execute(&mut *tx).await.map_err(database_failure)?;
            Some(id.to_string())
        } else {
            None
        };
        tx.commit().await.map_err(database_failure)?;
        Ok(ReadCollection {
            data_state: if scanned >= 2000 && next_cursor.is_some() {
                "PARTIAL"
            } else if items.is_empty() {
                "NO_DATA"
            } else {
                "FRESH"
            }
            .into(),
            items,
            next_cursor,
            snapshot_at,
            total_visible: None,
        })
    }

    pub async fn read_projection(
        &self,
        actor: &CommandActor,
        kind: &str,
        id: &str,
    ) -> Result<ResourceProjection, CommandFailure> {
        if !BASELINE_RESOURCE_TYPES.contains(&kind) && !EXTENSION_RESOURCE_TYPES.contains(&kind) {
            return Err(CommandFailure::not_found());
        }
        let mut tx = self.begin(actor, false).await?;
        let policy = self.read_policy(&mut tx, actor).await?;
        let row = sqlx::query("SELECT p.version,p.updated_at,a.id,a.digest,a.schema_version,a.classification FROM resource_projections p JOIN artifacts a ON a.tenant_id=p.tenant_id AND a.id=p.artifact_id WHERE p.tenant_id=$1 AND p.resource_type=$2 AND p.resource_id=$3 AND a.state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM deletion_tombstones d WHERE d.tenant_id=p.tenant_id AND d.resource_type=p.resource_type AND d.resource_id=p.resource_id) ORDER BY p.version DESC LIMIT 1")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(kind).bind(parse_uuid(id)?.to_string())
            .fetch_optional(&mut *tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let attrs = BTreeMap::from([
            (
                "classification".into(),
                json!(
                    row.try_get::<String, _>("classification")
                        .map_err(database_failure)?
                ),
            ),
            ("version".into(), json!(version)),
        ]);
        if !policy.permits("resource.read", kind, id, attrs) {
            return Err(CommandFailure::not_found());
        }
        let projection = ResourceProjection {
            resource: resource_ref(kind, parse_uuid(id)?, version),
            artifact_ref: artifact_reference(actor, &row)?,
            updated_at: row.try_get("updated_at").map_err(database_failure)?,
            data_state: "FRESH".into(),
        };
        tx.commit().await.map_err(database_failure)?;
        Ok(projection)
    }

    pub async fn artifact_metadata(
        &self,
        actor: &CommandActor,
        id: &str,
    ) -> Result<StoredArtifact, CommandFailure> {
        let mut tx = self.begin(actor, false).await?;
        let metadata = self.artifact_metadata_in(&mut tx, actor, id).await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(metadata)
    }

    pub(crate) async fn artifact_metadata_in(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        id: &str,
    ) -> Result<StoredArtifact, CommandFailure> {
        let row = sqlx::query("SELECT a.* FROM artifacts a WHERE a.tenant_id=$1 AND a.id=$2 AND a.state='ACTIVE' AND NOT EXISTS(SELECT 1 FROM deletion_tombstones d WHERE d.tenant_id=a.tenant_id AND d.resource_type='Artifact' AND d.resource_id=a.id::text)")
            .bind(parse_uuid(actor.tenant_id.as_str())?).bind(parse_uuid(id)?)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let policy = self.read_policy(tx, actor).await?;
        if !policy.permits(
            "artifact.read",
            "Artifact",
            id,
            BTreeMap::from([
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
                        row.try_get::<Uuid, _>("creator_id")
                            .map_err(database_failure)?
                            .to_string()
                    ),
                ),
            ]),
        ) {
            return Err(CommandFailure::not_found());
        }
        Ok(StoredArtifact {
            reference: artifact_reference(actor, &row)?,
            content_type: row.try_get("content_type").map_err(database_failure)?,
            size_bytes: row.try_get("size_bytes").map_err(database_failure)?,
            created_at: row.try_get("created_at").map_err(database_failure)?,
            state: row.try_get("state").map_err(database_failure)?,
        })
    }

    pub async fn artifact_content(
        &self,
        actor: &CommandActor,
        id: &str,
        maximum: usize,
    ) -> Result<(StoredArtifact, Vec<u8>), CommandFailure> {
        let mut tx = self.begin(actor, false).await?;
        let metadata = self.artifact_metadata_in(&mut tx, actor, id).await?;
        let bytes = self
            .load_artifact_ref(&mut tx, actor, &metadata.reference, maximum)
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok((metadata, bytes))
    }
}

fn artifact_reference(actor: &CommandActor, row: &PgRow) -> Result<ArtifactRef, CommandFailure> {
    let classification: Classification = serde_json::from_value(json!(
        row.try_get::<String, _>("classification")
            .map_err(database_failure)?
    ))
    .map_err(|_| CommandFailure::unavailable("ARTIFACT_METADATA_INVALID"))?;
    Ok(ArtifactRef {
        artifact_id: ArtifactId::new(
            row.try_get::<Uuid, _>("id")
                .map_err(database_failure)?
                .to_string(),
        )
        .expect("uuid"),
        tenant_id: actor.tenant_id.clone(),
        digest: Digest::parse(
            row.try_get::<String, _>("digest")
                .map_err(database_failure)?,
        )
        .map_err(|_| CommandFailure::unavailable("ARTIFACT_METADATA_INVALID"))?,
        schema_version: row.try_get("schema_version").map_err(database_failure)?,
        classification,
    })
}

fn snapshot_required() -> CommandFailure {
    CommandFailure::new("SNAPSHOT_REQUIRED", FailureKind::Expired)
}
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
