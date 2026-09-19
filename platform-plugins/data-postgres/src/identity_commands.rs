use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use masonwing_authorization_cedar::CedarPolicySnapshot;
use masonwing_contract_validation::{SchemaCatalog, digest_bytes, fingerprint};
use masonwing_contracts::wire::{ArtifactRef, CommandReceipt, ReceiptState, ResourceRef};
use masonwing_contracts::{Digest, PrincipalId, ResourceId, TenantId};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, FailureKind, require_expected_version,
};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{
    Mutation, PostgresStore, ProjectionChange, Tx, database_failure, field, parse_uuid,
    resource_id, resource_ref,
};

type HmacSha256 = Hmac<Sha256>;

const INVITE_TOKEN_VERSION: &str = "mwiv1";
const INVITE_TTL: Duration = Duration::hours(1);
const MAX_POLICY_BYTES: usize = 1024 * 1024;
const ALLOWED_ROLES: &[&str] = &[
    "OWNER",
    "EDITOR",
    "REVIEWER",
    "PUBLISHER",
    "VIEWER",
    "SERVICE",
    "SUPPORT",
];

/// Operator-provided HMAC key used only for invitation bearer tokens.
///
/// The key has no `Debug`, `Serialize`, or string conversion implementation.
/// Runtime bootstrap should construct it from a secret handle/environment value
/// and inject it into the command boundary; it is never stored in PostgreSQL.
#[derive(Clone)]
pub struct InviteTokenKey(Arc<[u8]>);

impl InviteTokenKey {
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, CommandFailure> {
        let bytes = bytes.as_ref();
        if !(32..=128).contains(&bytes.len()) {
            return Err(CommandFailure::invalid("INVITE_TOKEN_KEY_INVALID"));
        }
        Ok(Self(Arc::from(bytes)))
    }

    fn mac(&self, invite_id: Uuid, tenant_id: Uuid, expires_unix: i64) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(&self.0)
            .expect("validated HMAC key length is accepted by HMAC-SHA256");
        mac.update(INVITE_TOKEN_VERSION.as_bytes());
        mac.update(b"\0");
        mac.update(invite_id.as_bytes());
        mac.update(tenant_id.as_bytes());
        mac.update(&expires_unix.to_be_bytes());
        mac
    }

    fn derive_token(&self, invite_id: Uuid, tenant_id: Uuid, expires_at: DateTime<Utc>) -> String {
        let expires_unix = expires_at.timestamp();
        let signature = self
            .mac(invite_id, tenant_id, expires_unix)
            .finalize()
            .into_bytes();
        format!(
            "{INVITE_TOKEN_VERSION}.{invite_id}.{tenant_id}.{expires_unix}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        )
    }

    fn verify_token(&self, raw: &str) -> Result<InviteTokenClaims, CommandFailure> {
        if raw.len() > 256 || raw.chars().any(char::is_whitespace) {
            return Err(CommandFailure::denied("INVITE_TOKEN_INVALID"));
        }
        let mut parts = raw.split('.');
        if parts.next() != Some(INVITE_TOKEN_VERSION) {
            return Err(CommandFailure::denied("INVITE_TOKEN_INVALID"));
        }
        let invite_id = parts
            .next()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        let tenant_id = parts
            .next()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        let expires_unix = parts
            .next()
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or_else(|| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        let signature = parts
            .next()
            .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
            .ok_or_else(|| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        if parts.next().is_some() {
            return Err(CommandFailure::denied("INVITE_TOKEN_INVALID"));
        }
        self.mac(invite_id, tenant_id, expires_unix)
            .verify_slice(&signature)
            .map_err(|_| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        let expires_at = DateTime::<Utc>::from_timestamp(expires_unix, 0)
            .ok_or_else(|| CommandFailure::denied("INVITE_TOKEN_INVALID"))?;
        Ok(InviteTokenClaims {
            invite_id,
            tenant_id,
            expires_at,
        })
    }
}

struct InviteTokenClaims {
    invite_id: Uuid,
    tenant_id: Uuid,
    expires_at: DateTime<Utc>,
}

/// One-time bearer secret returned only from the dedicated no-store endpoint.
/// Deliberately not `Clone`, `Debug` or `Serialize`.
pub struct OneTimeInviteSecret(String);

impl OneTimeInviteSecret {
    pub fn expose(self) -> String {
        self.0
    }
}

/// Identity facts copied from an already verified server-side OIDC session.
/// The accept path rechecks all three against `oidc_identities` and obtains the
/// verified email from PostgreSQL; no browser-provided email is accepted.
#[derive(Clone)]
pub struct VerifiedInviteAcceptIdentity {
    pub principal_id: Uuid,
    pub issuer: String,
    pub subject: String,
}

/// Membership accept is the one bootstrap command which runs before a tenant
/// membership exists, so it cannot use the normal `AuthorizedCommand` actor.
pub struct MembershipAcceptBootstrapCommand {
    pub expected_tenant_id: Uuid,
    pub identity: VerifiedInviteAcceptIdentity,
    pub idempotency_key: ResourceId,
    pub fingerprint: Digest,
    pub input: Value,
}

impl PostgresStore {
    pub(crate) async fn invite_membership(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
        key: &InviteTokenKey,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let email = normalize_email(&field::<String>(&command.input, "email")?)?;
        let roles = normalize_roles(field(&command.input, "roles")?)?;
        self.require_assignable_roles(tx, &command.actor, &roles)
            .await?;
        let expires_at: DateTime<Utc> = field(&command.input, "expires_at")?;
        let expires_at = DateTime::<Utc>::from_timestamp(expires_at.timestamp(), 0)
            .ok_or_else(|| CommandFailure::invalid("INVITE_EXPIRY_INVALID"))?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        if expires_at <= now || expires_at > now + INVITE_TTL {
            return Err(CommandFailure::invalid("INVITE_EXPIRY_INVALID"));
        }

        let invite_id = Uuid::new_v4();
        let token = key.derive_token(invite_id, tenant, expires_at);
        let token_digest = digest_bytes(token.as_bytes());
        sqlx::query(
            r#"
            INSERT INTO membership_invites
                (tenant_id,invite_id,email_normalized,roles,expires_at,token_digest,created_by)
            VALUES($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(tenant)
        .bind(invite_id)
        .bind(&email)
        .bind(&roles)
        .bind(expires_at)
        .bind(token_digest.as_str())
        .bind(principal)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;

        Ok(Mutation::one(
            "MembershipInvitation",
            invite_id,
            1,
            json!({
                "invite_id": invite_id,
                "tenant_id": tenant,
                "email": email,
                "roles": roles,
                "expires_at": expires_at,
                "state": "PENDING",
                "version": 1
            }),
            "Membership invitation",
        ))
    }

    /// Retrieve an invite bearer secret exactly once. The row transition and an
    /// audit record commit atomically; raw token and token digest never enter the
    /// audit record, command receipt or resource projection.
    pub async fn retrieve_membership_invite_secret(
        &self,
        actor: &CommandActor,
        invite_id: Uuid,
        key: &InviteTokenKey,
    ) -> Result<OneTimeInviteSecret, CommandFailure> {
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let principal = parse_uuid(actor.principal_id.as_str())?;
        let mut tx = self.begin(actor, true).await?;
        let row = sqlx::query(
            r#"
            SELECT expires_at,token_digest,secret_retrieved_at,consumed_at,created_by
            FROM membership_invites
            WHERE tenant_id=$1 AND invite_id=$2
            FOR UPDATE
            "#,
        )
        .bind(tenant)
        .bind(invite_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(database_failure)?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
        if expires_at <= now {
            return Err(CommandFailure::new("INVITE_EXPIRED", FailureKind::Expired));
        }
        if row
            .try_get::<Option<DateTime<Utc>>, _>("consumed_at")
            .map_err(database_failure)?
            .is_some()
        {
            return Err(CommandFailure::conflict("INVITE_ALREADY_USED"));
        }
        if row
            .try_get::<Option<DateTime<Utc>>, _>("secret_retrieved_at")
            .map_err(database_failure)?
            .is_some()
        {
            return Err(CommandFailure::conflict("INVITE_SECRET_ALREADY_RETRIEVED"));
        }

        let retriever_roles = actor_membership_roles(&mut tx, actor).await?;
        let retriever_is_owner = retriever_roles.iter().any(|role| role == "OWNER");
        let retriever_is_editor = retriever_roles.iter().any(|role| role == "EDITOR");
        let created_by: Uuid = row.try_get("created_by").map_err(database_failure)?;
        if !retriever_is_owner && (!retriever_is_editor || created_by != principal) {
            return Err(CommandFailure::denied("MEMBERSHIP_INVITE_SECRET_DENIED"));
        }

        let token = key.derive_token(invite_id, tenant, expires_at);
        if row
            .try_get::<String, _>("token_digest")
            .map_err(database_failure)?
            != digest_bytes(token.as_bytes()).as_str()
        {
            return Err(CommandFailure::unavailable("INVITE_INTEGRITY"));
        }

        sqlx::query(
            "UPDATE membership_invites SET secret_retrieved_at=clock_timestamp() WHERE tenant_id=$1 AND invite_id=$2",
        )
        .bind(tenant)
        .bind(invite_id)
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;
        sqlx::query(
            "INSERT INTO audit_events(tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES($1,$2,$3,'membership.invite.secret.retrieve','MembershipInvitation',$4,$5)",
        )
        .bind(tenant)
        .bind(Uuid::new_v4())
        .bind(principal)
        .bind(invite_id)
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;
        tx.commit().await.map_err(database_failure)?;
        Ok(OneTimeInviteSecret(token))
    }

    /// Bootstrap acceptance path for an OIDC-authenticated principal that does
    /// not yet have tenant membership. It owns finalization because the normal
    /// command dispatcher intentionally requires an already-active membership.
    pub async fn execute_membership_accept_bootstrap(
        &self,
        command: MembershipAcceptBootstrapCommand,
        key: &InviteTokenKey,
    ) -> Result<CommandReceipt, CommandFailure> {
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
            .validate("Command_membership_accept", &command.input)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        if fingerprint("membership.accept", &command.input)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?
            != command.fingerprint
        {
            return Err(CommandFailure::invalid("FINGERPRINT_MISMATCH"));
        }
        let raw_token: String = field(&command.input, "invite_token")?;
        let claims = key.verify_token(&raw_token)?;
        if claims.tenant_id != command.expected_tenant_id {
            return Err(CommandFailure::denied("TENANT_CONTEXT_MISMATCH"));
        }

        let tenant = claims.tenant_id;
        let principal = command.identity.principal_id;
        let mut tx = self.pool.begin().await.map_err(database_failure)?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true),set_config('app.principal_id',$2,true),set_config('statement_timeout','10000',true),set_config('lock_timeout','5000',true)")
            .bind(tenant.to_string()).bind(principal.to_string())
            .execute(&mut *tx).await.map_err(database_failure)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("masonwing:tenant:{tenant}"))
            .execute(&mut *tx)
            .await
            .map_err(database_failure)?;
        let tenant_active: Option<bool> =
            sqlx::query_scalar("SELECT status='ACTIVE' FROM tenants WHERE id=$1")
                .bind(tenant)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_failure)?;
        if tenant_active != Some(true) {
            return Err(CommandFailure::not_found());
        }

        let identity = sqlx::query(
            "SELECT email,email_verified FROM oidc_identities WHERE principal_id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(principal)
        .bind(&command.identity.issuer)
        .bind(&command.identity.subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(|| CommandFailure::denied("INVITE_IDENTITY_MISMATCH"))?;
        let verified = identity
            .try_get::<bool, _>("email_verified")
            .map_err(database_failure)?;
        let verified_email = identity
            .try_get::<Option<String>, _>("email")
            .map_err(database_failure)?
            .filter(|_| verified)
            .ok_or_else(|| CommandFailure::denied("INVITE_EMAIL_NOT_VERIFIED"))?;
        let verified_email = normalize_email(&verified_email)?;

        if let Some(previous) = sqlx::query(
            "SELECT fingerprint,receipt FROM command_receipts WHERE tenant_id=$1 AND principal_id=$2 AND operation='membership.accept' AND idempotency_key=$3 AND expires_at>clock_timestamp() FOR UPDATE",
        )
        .bind(tenant)
        .bind(principal)
        .bind(command.idempotency_key.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        {
            if previous
                .try_get::<String, _>("fingerprint")
                .map_err(database_failure)?
                != command.fingerprint.as_str()
            {
                return Err(CommandFailure::conflict("IDEMPOTENCY_CONFLICT"));
            }
            let current_active: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM memberships WHERE tenant_id=$1 AND principal_id=$2 AND status='ACTIVE')",
            )
            .bind(tenant)
            .bind(principal)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
            if !current_active {
                return Err(CommandFailure::denied("AUTHORITY_CHANGED"));
            }
            let receipt_value: Value = previous.try_get("receipt").map_err(database_failure)?;
            SchemaCatalog::shared()
                .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
                .validate("CommandReceipt", &receipt_value)
                .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
            let receipt = serde_json::from_value(receipt_value)
                .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
            tx.commit().await.map_err(database_failure)?;
            return Ok(receipt);
        }

        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;
        if claims.expires_at <= now {
            return Err(CommandFailure::new("INVITE_EXPIRED", FailureKind::Expired));
        }
        let invite = sqlx::query(
            r#"
            SELECT email_normalized,roles,expires_at,token_digest,consumed_at
            FROM membership_invites
            WHERE tenant_id=$1 AND invite_id=$2
            FOR UPDATE
            "#,
        )
        .bind(tenant)
        .bind(claims.invite_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        let invite_expiry: DateTime<Utc> =
            invite.try_get("expires_at").map_err(database_failure)?;
        if invite_expiry.timestamp() != claims.expires_at.timestamp() || invite_expiry <= now {
            return Err(CommandFailure::new("INVITE_EXPIRED", FailureKind::Expired));
        }
        if invite
            .try_get::<Option<DateTime<Utc>>, _>("consumed_at")
            .map_err(database_failure)?
            .is_some()
        {
            return Err(CommandFailure::conflict("INVITE_ALREADY_USED"));
        }
        if invite
            .try_get::<String, _>("token_digest")
            .map_err(database_failure)?
            != digest_bytes(raw_token.as_bytes()).as_str()
        {
            return Err(CommandFailure::denied("INVITE_TOKEN_INVALID"));
        }
        if invite
            .try_get::<String, _>("email_normalized")
            .map_err(database_failure)?
            != verified_email
        {
            return Err(CommandFailure::denied("INVITE_EMAIL_MISMATCH"));
        }
        let roles = normalize_roles(invite.try_get("roles").map_err(database_failure)?)?;
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE tenant_id=$1 AND principal_id=$2)",
        )
        .bind(tenant)
        .bind(principal)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_failure)?;
        if existing {
            return Err(CommandFailure::conflict("MEMBERSHIP_EXISTS"));
        }

        let membership_id = Uuid::new_v4();
        let primary_role = canonical_primary_role(&roles)?;
        sqlx::query(
            "INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch,version) VALUES($1,$2,$3,$4,'ACTIVE',1,1,1)",
        )
        .bind(tenant)
        .bind(principal)
        .bind(membership_id)
        .bind(primary_role)
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;
        insert_roles(&mut tx, tenant, membership_id, &roles).await?;
        sqlx::query(
            "UPDATE membership_invites SET consumed_at=clock_timestamp(),consumed_by=$3,membership_id=$4,version=version+1 WHERE tenant_id=$1 AND invite_id=$2",
        )
        .bind(tenant)
        .bind(claims.invite_id)
        .bind(principal)
        .bind(membership_id)
        .execute(&mut *tx)
        .await
        .map_err(database_failure)?;

        let policy = sqlx::query(
            "SELECT policy_version,policy_epoch FROM authorization_policies WHERE tenant_id=$1 AND is_current FOR SHARE",
        )
        .bind(tenant)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(|| CommandFailure::unavailable("POLICY_UNAVAILABLE"))?;
        let actor = CommandActor {
            tenant_id: TenantId::new(tenant.to_string())
                .map_err(|_| CommandFailure::unavailable("IDENTITY_INTEGRITY"))?,
            principal_id: PrincipalId::new(principal.to_string())
                .map_err(|_| CommandFailure::unavailable("IDENTITY_INTEGRITY"))?,
            issuer: command.identity.issuer.clone(),
            membership_epoch: 1,
            permission_epoch: 1,
            policy_version: policy.try_get("policy_version").map_err(database_failure)?,
            policy_epoch: policy.try_get("policy_epoch").map_err(database_failure)?,
        };
        let membership_change = ProjectionChange {
            resource: resource_ref("Membership", membership_id, 1),
            value: membership_projection_value(
                membership_id,
                tenant,
                principal,
                &roles,
                "ACTIVE",
                1,
            ),
            label: "Membership".into(),
            source_artifacts: Vec::new(),
        };
        let invite_change = ProjectionChange {
            resource: resource_ref("MembershipInvitation", claims.invite_id, 2),
            value: json!({
                "invite_id": claims.invite_id,
                "tenant_id": tenant,
                "email": verified_email,
                "roles": roles,
                "expires_at": invite_expiry,
                "state": "CONSUMED",
                "membership_id": membership_id,
                "version": 2
            }),
            label: "Consumed membership invitation".into(),
            source_artifacts: Vec::new(),
        };
        let mutation = Mutation {
            primary: membership_change.resource.clone(),
            changes: vec![membership_change, invite_change],
            run_id: None,
            effect_id: None,
            accepted: false,
        };
        let receipt = self
            .finalize_bootstrap_accept(&mut tx, &actor, &command, mutation, tenant, principal)
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(receipt)
    }

    pub(crate) async fn change_membership(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let membership_id = parse_uuid(&field::<String>(&command.input, "membership_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let roles = normalize_roles(field(&command.input, "roles")?)?;
        self.require_assignable_roles(tx, &command.actor, &roles)
            .await?;

        let target = sqlx::query(
            "SELECT principal_id,status,version,membership_epoch,permission_epoch FROM memberships WHERE tenant_id=$1 AND membership_id=$2 FOR UPDATE",
        )
        .bind(tenant)
        .bind(membership_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        if target
            .try_get::<String, _>("status")
            .map_err(database_failure)?
            != "ACTIVE"
        {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let version = require_expected_version(
            target.try_get("version").map_err(database_failure)?,
            expected,
        )?;
        let had_owner = membership_has_role(tx, tenant, membership_id, "OWNER").await?;
        require_actor_can_manage_owner(tx, &command.actor, had_owner).await?;
        if had_owner && !roles.iter().any(|role| role == "OWNER") {
            ensure_not_last_owner(tx, tenant).await?;
        }

        sqlx::query("DELETE FROM membership_roles WHERE tenant_id=$1 AND membership_id=$2")
            .bind(tenant)
            .bind(membership_id)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        insert_roles(tx, tenant, membership_id, &roles).await?;
        let primary_role = canonical_primary_role(&roles)?;
        let updated = sqlx::query(
            "UPDATE memberships SET role=$3,version=$4,membership_epoch=membership_epoch+1,permission_epoch=permission_epoch+1 WHERE tenant_id=$1 AND membership_id=$2 RETURNING principal_id",
        )
        .bind(tenant)
        .bind(membership_id)
        .bind(primary_role)
        .bind(version)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_failure)?;

        Ok(Mutation::one(
            "Membership",
            membership_id,
            version,
            membership_projection_value(
                membership_id,
                tenant,
                updated
                    .try_get::<Uuid, _>("principal_id")
                    .map_err(database_failure)?,
                &roles,
                "ACTIVE",
                version,
            ),
            "Membership",
        ))
    }

    pub(crate) async fn revoke_membership(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let membership_id = parse_uuid(&field::<String>(&command.input, "membership_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let target = sqlx::query(
            "SELECT principal_id,status,version FROM memberships WHERE tenant_id=$1 AND membership_id=$2 FOR UPDATE",
        )
        .bind(tenant)
        .bind(membership_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        if target
            .try_get::<String, _>("status")
            .map_err(database_failure)?
            != "ACTIVE"
        {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let version = require_expected_version(
            target.try_get("version").map_err(database_failure)?,
            expected,
        )?;
        let target_is_owner = membership_has_role(tx, tenant, membership_id, "OWNER").await?;
        require_actor_can_manage_owner(tx, &command.actor, target_is_owner).await?;
        if target_is_owner {
            ensure_not_last_owner(tx, tenant).await?;
        }
        let roles = membership_roles(tx, tenant, membership_id).await?;
        let updated = sqlx::query(
            "UPDATE memberships SET status='REVOKED',version=$3,membership_epoch=membership_epoch+1,permission_epoch=permission_epoch+1 WHERE tenant_id=$1 AND membership_id=$2 RETURNING principal_id",
        )
        .bind(tenant)
        .bind(membership_id)
        .bind(version)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_failure)?;
        Ok(Mutation::one(
            "Membership",
            membership_id,
            version,
            membership_projection_value(
                membership_id,
                tenant,
                updated
                    .try_get::<Uuid, _>("principal_id")
                    .map_err(database_failure)?,
                &roles,
                "REVOKED",
                version,
            ),
            "Revoked membership",
        ))
    }

    pub(crate) async fn evaluate_policy(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let action: String = field(&command.input, "action")?;
        let resource: ResourceRef = field(&command.input, "resource")?;
        let row = sqlx::query(
            "SELECT p.version,a.classification FROM resource_projections p JOIN artifacts a ON a.tenant_id=p.tenant_id AND a.id=p.artifact_id WHERE p.tenant_id=$1 AND p.resource_type=$2 AND p.resource_id=$3 AND a.state='ACTIVE' ORDER BY p.version DESC LIMIT 1",
        )
        .bind(tenant)
        .bind(&resource.resource_type)
        .bind(resource.resource_id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        let current_version: i32 = row.try_get("version").map_err(database_failure)?;
        if current_version != resource.version {
            return Err(CommandFailure::conflict("STALE_VERSION"));
        }
        let policy = self.read_policy(tx, &command.actor).await?;
        let allowed = policy.permits(
            &action,
            &resource.resource_type,
            resource.resource_id.as_str(),
            std::collections::BTreeMap::from([
                ("version".into(), json!(current_version)),
                (
                    "classification".into(),
                    json!(
                        row.try_get::<String, _>("classification")
                            .map_err(database_failure)?
                    ),
                ),
            ]),
        );
        let id = Uuid::new_v4();
        Ok(Mutation::one(
            "PolicyDecision",
            id,
            1,
            json!({
                "decision_id": id,
                "action": action,
                "resource": resource,
                "decision": if allowed { "ALLOW" } else { "DENY" },
                "policy_version": command.actor.policy_version,
                "policy_epoch": command.actor.policy_epoch,
                "version": 1
            }),
            "Policy evaluation",
        ))
    }

    pub(crate) async fn propose_policy(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let policy_ref: ArtifactRef = field(&command.input, "policy_ref")?;
        let review_ref: ArtifactRef = field(&command.input, "review_ref")?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let policy_bytes = self
            .load_artifact_ref(tx, &command.actor, &policy_ref, MAX_POLICY_BYTES)
            .await?;
        // Review evidence is immutable and digest-checked even though its bytes
        // are not interpreted by this storage adapter.
        self.load_artifact_ref(tx, &command.actor, &review_ref, MAX_POLICY_BYTES)
            .await?;
        let cedar_source = std::str::from_utf8(&policy_bytes)
            .map_err(|_| CommandFailure::invalid("POLICY_SOURCE_INVALID"))?;
        if cedar_source.trim().is_empty() {
            return Err(CommandFailure::invalid("POLICY_SOURCE_INVALID"));
        }
        CedarPolicySnapshot::parse("0.0.0", 1, cedar_source)
            .map_err(|_| CommandFailure::invalid("POLICY_SOURCE_INVALID"))?;
        sqlx::query(
            "INSERT INTO policy_configuration_state(tenant_id,version) VALUES($1,1) ON CONFLICT(tenant_id) DO NOTHING",
        )
        .bind(tenant)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        let current: i32 = sqlx::query_scalar(
            "SELECT version FROM policy_configuration_state WHERE tenant_id=$1 FOR UPDATE",
        )
        .bind(tenant)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_failure)?;
        let version = require_expected_version(current, expected)?;
        sqlx::query(
            "UPDATE policy_configuration_state SET version=$2,updated_at=clock_timestamp() WHERE tenant_id=$1",
        )
        .bind(tenant)
        .bind(version)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        let current_policy = sqlx::query(
            "SELECT policy_version,policy_epoch FROM authorization_policies WHERE tenant_id=$1 AND is_current FOR SHARE",
        )
        .bind(tenant)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(|| CommandFailure::unavailable("POLICY_UNAVAILABLE"))?;
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO policy_proposals
                (tenant_id,id,policy_ref,review_ref,policy_digest,cedar_source,
                 base_policy_version,base_policy_epoch,state,created_by,version)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,'PENDING',$9,$10)
            "#,
        )
        .bind(tenant)
        .bind(id)
        .bind(
            serde_json::to_value(&policy_ref)
                .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?,
        )
        .bind(
            serde_json::to_value(&review_ref)
                .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?,
        )
        .bind(policy_ref.digest.as_str())
        .bind(cedar_source)
        .bind(
            current_policy
                .try_get::<String, _>("policy_version")
                .map_err(database_failure)?,
        )
        .bind(
            current_policy
                .try_get::<i64, _>("policy_epoch")
                .map_err(database_failure)?,
        )
        .bind(principal)
        .bind(version)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        Ok(Mutation::one(
            "PolicyProposal",
            id,
            version,
            json!({
                "proposal_id": id,
                "policy_ref": policy_ref,
                "review_ref": review_ref,
                "state": "PENDING",
                "base_policy_version": command.actor.policy_version,
                "base_policy_epoch": command.actor.policy_epoch,
                "version": version
            }),
            "Pending policy proposal",
        ))
    }

    pub(crate) async fn configure_identity(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let issuer: String = field(&command.input, "issuer")?;
        let client_id: String = field(&command.input, "client_id")?;
        let client_secret_ref: String = field(&command.input, "client_secret_ref")?;
        let mut redirect_uris: Vec<String> = field(&command.input, "allowed_redirect_uris")?;
        redirect_uris.sort();
        redirect_uris.dedup();
        if issuer.trim().is_empty()
            || client_id.trim().is_empty()
            || client_secret_ref.trim().is_empty()
        {
            return Err(CommandFailure::invalid("IDENTITY_CONFIGURATION_INVALID"));
        }
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO identity_configuration_proposals
                (tenant_id,id,issuer,client_id,client_secret_ref,allowed_redirect_uris,
                 state,runtime_state,created_by,version)
            VALUES($1,$2,$3,$4,$5,$6,'PENDING','UNQUALIFIED',$7,1)
            "#,
        )
        .bind(tenant)
        .bind(id)
        .bind(&issuer)
        .bind(&client_id)
        .bind(&client_secret_ref)
        .bind(&redirect_uris)
        .bind(principal)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        Ok(Mutation::one(
            "IdentityConfigurationProposal",
            id,
            1,
            json!({
                "proposal_id": id,
                "issuer": issuer,
                "client_id": client_id,
                "client_secret_ref": client_secret_ref,
                "allowed_redirect_uris": redirect_uris,
                "state": "PENDING",
                "runtime_state": "UNQUALIFIED",
                "version": 1
            }),
            "Pending identity configuration",
        ))
    }

    async fn require_assignable_roles(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        requested: &[String],
    ) -> Result<(), CommandFailure> {
        let actor_roles = actor_membership_roles(tx, actor).await?;
        if actor_roles.iter().any(|role| role == "OWNER") {
            return Ok(());
        }
        if requested.iter().any(|role| role == "OWNER")
            || requested.iter().any(|role| !actor_roles.contains(role))
        {
            return Err(CommandFailure::denied("MEMBERSHIP_ROLE_ESCALATION"));
        }
        Ok(())
    }

    async fn finalize_bootstrap_accept(
        &self,
        tx: &mut Tx<'_>,
        actor: &CommandActor,
        command: &MembershipAcceptBootstrapCommand,
        mutation: Mutation,
        tenant: Uuid,
        principal: Uuid,
    ) -> Result<CommandReceipt, CommandFailure> {
        let command_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let accepted_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        for change in &mutation.changes {
            let artifact = self.write_projection(tx, actor, change).await?;
            sqlx::query("INSERT INTO audit_events(tenant_id,id,principal_id,action,resource_type,resource_id,correlation_id) VALUES($1,$2,$3,'membership.accept',$4,$5,$6)")
                .bind(tenant).bind(Uuid::new_v4()).bind(principal)
                .bind(&change.resource.resource_type)
                .bind(parse_uuid(change.resource.resource_id.as_str())?)
                .bind(correlation_id).execute(&mut **tx).await.map_err(database_failure)?;
            sqlx::query("INSERT INTO outbox_events(tenant_id,id,aggregate_id,aggregate_version,event_name,artifact_refs,aggregate_type,principal_id,correlation_id) VALUES($1,$2,$3,$4,'membership.accept',$5,$6,$7,$8)")
                .bind(tenant).bind(Uuid::new_v4()).bind(parse_uuid(change.resource.resource_id.as_str())?)
                .bind(change.resource.version).bind(json!([artifact]))
                .bind(&change.resource.resource_type).bind(principal).bind(correlation_id)
                .execute(&mut **tx).await.map_err(database_failure)?;
        }
        let receipt = CommandReceipt {
            command_id: resource_id(command_id),
            state: ReceiptState::Succeeded,
            resource: Some(mutation.primary),
            run_id: None,
            effect_id: None,
            correlation_id: resource_id(correlation_id),
            accepted_at,
        };
        let value = serde_json::to_value(&receipt)
            .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
            .validate("CommandReceipt", &value)
            .map_err(|_| CommandFailure::unavailable("RECEIPT_INTEGRITY"))?;
        sqlx::query("INSERT INTO command_receipts(tenant_id,principal_id,operation,idempotency_key,fingerprint,command_id,receipt,expires_at) VALUES($1,$2,'membership.accept',$3,$4,$5,$6,clock_timestamp()+interval '24 hours')")
            .bind(tenant).bind(principal).bind(command.idempotency_key.as_str())
            .bind(command.fingerprint.as_str()).bind(command_id).bind(value)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(receipt)
    }
}

async fn actor_membership_roles(
    tx: &mut Tx<'_>,
    actor: &CommandActor,
) -> Result<Vec<String>, CommandFailure> {
    let roles = sqlx::query_scalar::<_, String>(
        r#"
        SELECT mr.role
        FROM memberships m
        JOIN membership_roles mr
          ON mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id
        WHERE m.tenant_id=$1 AND m.principal_id=$2 AND m.status='ACTIVE'
        ORDER BY mr.role
        "#,
    )
    .bind(parse_uuid(actor.tenant_id.as_str())?)
    .bind(parse_uuid(actor.principal_id.as_str())?)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_failure)?;
    if roles.is_empty() {
        return Err(CommandFailure::denied("AUTHORITY_CHANGED"));
    }
    Ok(roles)
}

async fn require_actor_can_manage_owner(
    tx: &mut Tx<'_>,
    actor: &CommandActor,
    target_is_owner: bool,
) -> Result<(), CommandFailure> {
    if !target_is_owner {
        return Ok(());
    }
    let roles = actor_membership_roles(tx, actor).await?;
    if !roles.iter().any(|role| role == "OWNER") {
        return Err(CommandFailure::denied("MEMBERSHIP_OWNER_MANAGEMENT_DENIED"));
    }
    Ok(())
}

async fn membership_roles(
    tx: &mut Tx<'_>,
    tenant: Uuid,
    membership_id: Uuid,
) -> Result<Vec<String>, CommandFailure> {
    let roles = sqlx::query_scalar::<_, String>(
        "SELECT role FROM membership_roles WHERE tenant_id=$1 AND membership_id=$2 ORDER BY role",
    )
    .bind(tenant)
    .bind(membership_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_failure)?;
    if roles.is_empty() {
        return Err(CommandFailure::unavailable("MEMBERSHIP_ROLE_INTEGRITY"));
    }
    Ok(roles)
}

async fn membership_has_role(
    tx: &mut Tx<'_>,
    tenant: Uuid,
    membership_id: Uuid,
    role: &str,
) -> Result<bool, CommandFailure> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM membership_roles WHERE tenant_id=$1 AND membership_id=$2 AND role=$3)",
    )
    .bind(tenant)
    .bind(membership_id)
    .bind(role)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_failure)
}

async fn ensure_not_last_owner(tx: &mut Tx<'_>, tenant: Uuid) -> Result<(), CommandFailure> {
    let owners: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*)
        FROM memberships m
        WHERE m.tenant_id=$1 AND m.status='ACTIVE'
          AND EXISTS (
            SELECT 1 FROM membership_roles mr
            WHERE mr.tenant_id=m.tenant_id
              AND mr.membership_id=m.membership_id
              AND mr.role='OWNER'
          )
        "#,
    )
    .bind(tenant)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_failure)?;
    if owners <= 1 {
        return Err(CommandFailure::conflict("LAST_OWNER"));
    }
    Ok(())
}

async fn insert_roles(
    tx: &mut Tx<'_>,
    tenant: Uuid,
    membership_id: Uuid,
    roles: &[String],
) -> Result<(), CommandFailure> {
    for role in roles {
        sqlx::query("INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES($1,$2,$3)")
            .bind(tenant)
            .bind(membership_id)
            .bind(role)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
    }
    Ok(())
}

fn normalize_roles(mut roles: Vec<String>) -> Result<Vec<String>, CommandFailure> {
    if roles.is_empty() || roles.len() > 10 {
        return Err(CommandFailure::invalid("MEMBERSHIP_ROLES_INVALID"));
    }
    if roles
        .iter()
        .any(|role| !ALLOWED_ROLES.contains(&role.as_str()))
    {
        return Err(CommandFailure::invalid("MEMBERSHIP_ROLES_INVALID"));
    }
    let before = roles.len();
    roles.sort();
    roles.dedup();
    if roles.len() != before {
        return Err(CommandFailure::invalid("MEMBERSHIP_ROLES_INVALID"));
    }
    Ok(roles)
}

fn canonical_primary_role(roles: &[String]) -> Result<&str, CommandFailure> {
    roles
        .first()
        .map(String::as_str)
        .ok_or_else(|| CommandFailure::invalid("MEMBERSHIP_ROLES_INVALID"))
}

fn normalize_email(email: &str) -> Result<String, CommandFailure> {
    let normalized = email.trim().to_ascii_lowercase();
    if !(3..=320).contains(&normalized.len())
        || normalized.chars().any(char::is_whitespace)
        || !normalized.contains('@')
    {
        return Err(CommandFailure::invalid("INVITE_EMAIL_INVALID"));
    }
    Ok(normalized)
}

fn membership_projection_value(
    membership_id: Uuid,
    tenant_id: Uuid,
    principal_id: Uuid,
    roles: &[String],
    state: &str,
    version: i32,
) -> Value {
    json!({
        "membership_id": membership_id,
        "tenant_id": tenant_id,
        "principal_id": principal_id,
        "roles": roles,
        "state": state,
        "version": version
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_token_is_bound_to_invite_tenant_and_expiry() {
        let key = InviteTokenKey::from_bytes([7_u8; 32]).unwrap();
        let invite = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let expiry = DateTime::<Utc>::from_timestamp(2_000_000_000, 0).unwrap();
        let token = key.derive_token(invite, tenant, expiry);
        let claims = key.verify_token(&token).unwrap();
        assert_eq!(claims.invite_id, invite);
        assert_eq!(claims.tenant_id, tenant);
        assert_eq!(claims.expires_at, expiry);
        assert!(token.len() <= 256);
        SchemaCatalog::shared()
            .unwrap()
            .validate("Command_membership_accept", &json!({"invite_token": token}))
            .unwrap();

        let other_tenant = Uuid::new_v4();
        let tampered = token.replace(&tenant.to_string(), &other_tenant.to_string());
        assert_eq!(
            key.verify_token(&tampered)
                .err()
                .expect("tampered token denied")
                .code,
            "INVITE_TOKEN_INVALID"
        );
    }

    #[test]
    fn invite_key_and_role_validation_fail_closed() {
        assert_eq!(
            InviteTokenKey::from_bytes([0_u8; 16])
                .err()
                .expect("short invite key denied")
                .code,
            "INVITE_TOKEN_KEY_INVALID"
        );
        assert_eq!(
            normalize_roles(vec!["OWNER".into(), "OWNER".into()])
                .unwrap_err()
                .code,
            "MEMBERSHIP_ROLES_INVALID"
        );
        assert_eq!(
            normalize_roles(vec!["ROOT".into()]).unwrap_err().code,
            "MEMBERSHIP_ROLES_INVALID"
        );
    }

    #[test]
    fn canonical_primary_role_is_deterministic_and_not_authority_union() {
        let roles = normalize_roles(vec!["VIEWER".into(), "OWNER".into()]).unwrap();
        assert_eq!(roles, vec!["OWNER".to_owned(), "VIEWER".to_owned()]);
        assert_eq!(canonical_primary_role(&roles).unwrap(), "OWNER");
    }

    #[test]
    fn membership_projection_matches_immutable_contract_exactly() {
        use std::collections::BTreeSet;

        let value = membership_projection_value(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            &["EDITOR".into(), "VIEWER".into()],
            "ACTIVE",
            3,
        );
        SchemaCatalog::shared()
            .unwrap()
            .validate("Membership", &value)
            .unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "membership_id".to_owned(),
                "tenant_id".to_owned(),
                "principal_id".to_owned(),
                "roles".to_owned(),
                "state".to_owned(),
                "version".to_owned(),
            ])
        );
    }

    #[test]
    fn revoked_membership_projection_matches_contract() {
        let value = membership_projection_value(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            &["VIEWER".into()],
            "REVOKED",
            4,
        );
        SchemaCatalog::shared()
            .unwrap()
            .validate("Membership", &value)
            .unwrap();
    }
}
