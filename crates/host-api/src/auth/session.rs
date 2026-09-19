use std::cmp;

use axum::http::{HeaderMap, Uri, header::ORIGIN};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use time::Duration as CookieDuration;
use tower_sessions::{ExpiredDeletion, Expiry, Session, SessionManagerLayer, cookie::SameSite};
use tower_sessions_sqlx_store::PostgresStore;
use uuid::Uuid;

use masonwing_identity_oidc::{VerifiedOidcPrincipal, VerifiedStepUpEvidence};

use super::error::AuthError;

const AUTH_SESSION_KEY: &str = "masonwing.auth.v1";
const OIDC_BROWSER_FLOW_KEY: &str = "masonwing.oidc.browser_flow.v1";
pub const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionCookieProfile {
    /// Production/default profile: `__Host-` prefix, Secure, HttpOnly,
    /// SameSite=Lax, Path=/ and no Domain attribute.
    #[default]
    Secure,
    /// Explicit localhost development profile. This intentionally drops the
    /// `__Host-` prefix because browsers require Secure for that prefix.
    LoopbackDev,
}

#[derive(Clone)]
pub struct SessionAuthority {
    pool: PgPool,
    idle_timeout: ChronoDuration,
    absolute_timeout: ChronoDuration,
    step_up_timeout: ChronoDuration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionEnvelope {
    principal_id: Uuid,
    issuer: String,
    subject: String,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
    step_up_expires_at: Option<DateTime<Utc>>,
    csrf_token: String,
    active_tenant_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedSession {
    pub principal_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub absolute_expires_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub step_up_expires_at: Option<DateTime<Utc>>,
    pub csrf_token: String,
    pub active_tenant_id: Option<Uuid>,
}

#[derive(Clone, Debug, FromRow, PartialEq, Eq)]
pub struct CurrentMembership {
    pub tenant_id: Uuid,
    pub membership_id: Uuid,
    pub role: String,
    pub roles: Vec<String>,
    pub membership_epoch: i64,
    pub permission_epoch: i64,
}

#[derive(Clone, Debug, FromRow, PartialEq, Eq)]
pub struct CurrentPolicySource {
    pub policy_version: String,
    pub policy_epoch: i64,
    pub cedar_source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentAuthority {
    pub membership: CurrentMembership,
    pub policy: CurrentPolicySource,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PrincipalRef {
    #[serde(rename = "type")]
    pub principal_type: &'static str,
    pub id: String,
    pub issuer: String,
}

/// Exact MASONWING@1.0.1 OpenAPI `SessionView` shape.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SessionView {
    pub authenticated: bool,
    pub principal: Option<PrincipalRef>,
    pub tenant_ids: Vec<String>,
    pub active_tenant_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub step_up_expires_at: Option<DateTime<Utc>>,
    pub csrf_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingOidcFlow {
    pub(crate) browser_binding: String,
    pub(crate) purpose: OidcFlowPurpose,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum OidcFlowPurpose {
    Login,
    StepUp {
        principal_id: Uuid,
        issuer: String,
        subject: String,
    },
}

impl SessionAuthority {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            idle_timeout: ChronoDuration::minutes(30),
            absolute_timeout: ChronoDuration::hours(12),
            step_up_timeout: ChronoDuration::minutes(5),
        }
    }

    pub fn with_timeouts(
        mut self,
        idle_timeout: ChronoDuration,
        absolute_timeout: ChronoDuration,
        step_up_timeout: ChronoDuration,
    ) -> Result<Self, AuthError> {
        if idle_timeout <= ChronoDuration::zero()
            || absolute_timeout <= ChronoDuration::zero()
            || step_up_timeout <= ChronoDuration::zero()
        {
            return Err(AuthError::DependencyUnavailable);
        }
        self.idle_timeout = idle_timeout;
        self.absolute_timeout = absolute_timeout;
        self.step_up_timeout = step_up_timeout;
        Ok(self)
    }

    pub fn session_store(&self) -> PostgresStore {
        PostgresStore::new(self.pool.clone())
            .with_schema_name("tower_sessions")
            .expect("static tower_sessions schema name is valid")
            .with_table_name("session")
            .expect("static tower session table name is valid")
    }

    pub fn session_layer(
        &self,
        profile: SessionCookieProfile,
    ) -> SessionManagerLayer<PostgresStore> {
        let (name, secure) = match profile {
            SessionCookieProfile::Secure => ("__Host-masonwing_session", true),
            SessionCookieProfile::LoopbackDev => ("masonwing_session", false),
        };
        SessionManagerLayer::new(self.session_store())
            .with_name(name)
            .with_secure(secure)
            .with_http_only(true)
            .with_same_site(SameSite::Lax)
            .with_path("/")
            .with_expiry(Expiry::OnInactivity(CookieDuration::seconds(
                self.idle_timeout.num_seconds(),
            )))
    }

    pub async fn cleanup_expired_sessions(&self) -> Result<(), AuthError> {
        self.session_store()
            .delete_expired()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)
    }

    pub async fn set_oidc_login_binding(
        &self,
        session: &Session,
        binding: &str,
    ) -> Result<(), AuthError> {
        session
            .insert(
                OIDC_BROWSER_FLOW_KEY,
                &PendingOidcFlow {
                    browser_binding: binding.to_owned(),
                    purpose: OidcFlowPurpose::Login,
                },
            )
            .await
            .map_err(|_| AuthError::DependencyUnavailable)
    }

    pub async fn set_oidc_step_up_binding(
        &self,
        session: &Session,
        binding: &str,
        auth: &AuthenticatedSession,
    ) -> Result<(), AuthError> {
        session
            .insert(
                OIDC_BROWSER_FLOW_KEY,
                &PendingOidcFlow {
                    browser_binding: binding.to_owned(),
                    purpose: OidcFlowPurpose::StepUp {
                        principal_id: auth.principal_id,
                        issuer: auth.issuer.clone(),
                        subject: auth.subject.clone(),
                    },
                },
            )
            .await
            .map_err(|_| AuthError::DependencyUnavailable)
    }

    pub(crate) async fn pending_oidc_flow(
        &self,
        session: &Session,
    ) -> Result<PendingOidcFlow, AuthError> {
        session
            .get::<PendingOidcFlow>(OIDC_BROWSER_FLOW_KEY)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?
            .ok_or(AuthError::InvalidCallback)
    }

    pub async fn clear_oidc_browser_flow(&self, session: &Session) -> Result<(), AuthError> {
        session
            .remove::<PendingOidcFlow>(OIDC_BROWSER_FLOW_KEY)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(())
    }

    pub async fn establish_authenticated_session(
        &self,
        session: &Session,
        principal: &VerifiedOidcPrincipal,
    ) -> Result<AuthenticatedSession, AuthError> {
        let now = Utc::now();
        let memberships = self
            .list_current_memberships(principal.principal_id)
            .await?;
        let envelope = SessionEnvelope {
            principal_id: principal.principal_id,
            issuer: principal.issuer.clone(),
            subject: principal.subject.clone(),
            created_at: now,
            last_seen_at: now,
            absolute_expires_at: now + self.absolute_timeout,
            step_up_expires_at: None,
            csrf_token: random_session_secret(),
            active_tenant_id: memberships.first().map(|m| m.tenant_id),
        };

        // Discard all anonymous transaction state and rotate the ID across the
        // authentication privilege boundary (REQ-034/AC-035).
        session.clear().await;
        session
            .insert(AUTH_SESSION_KEY, &envelope)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        session
            .cycle_id()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(self.project(&envelope))
    }

    /// Read and expiry-check authentication without extending inactivity. Use
    /// this before CSRF validation on unsafe cookie-authenticated requests.
    pub async fn peek_authenticated(
        &self,
        session: &Session,
    ) -> Result<AuthenticatedSession, AuthError> {
        let envelope = self.read_envelope(session).await?;
        self.ensure_not_expired(&envelope, Utc::now())?;
        Ok(self.project(&envelope))
    }

    /// Read authentication and renew the inactivity window after validation.
    pub async fn touch_authenticated(
        &self,
        session: &Session,
    ) -> Result<AuthenticatedSession, AuthError> {
        let now = Utc::now();
        let mut envelope = self.read_envelope(session).await?;
        self.ensure_not_expired(&envelope, now)?;
        envelope.last_seen_at = now;
        session
            .insert(AUTH_SESSION_KEY, &envelope)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(self.project(&envelope))
    }

    pub async fn record_verified_step_up(
        &self,
        session: &Session,
        proof: &VerifiedStepUpEvidence,
    ) -> Result<AuthenticatedSession, AuthError> {
        let now = Utc::now();
        let mut envelope = self.read_envelope(session).await?;
        self.ensure_not_expired(&envelope, now)?;
        if proof.principal_id() != envelope.principal_id
            || proof.issuer() != envelope.issuer
            || proof.subject() != envelope.subject
            || proof.verified_at() > now
            || now - proof.verified_at() > ChronoDuration::minutes(1)
        {
            return Err(AuthError::StepUpRequired);
        }
        envelope.step_up_expires_at = Some(proof.verified_at() + self.step_up_timeout);
        envelope.last_seen_at = now;
        session
            .insert(AUTH_SESSION_KEY, &envelope)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        session
            .cycle_id()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(self.project(&envelope))
    }

    pub async fn logout(&self, session: &Session) -> Result<(), AuthError> {
        session
            .flush()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)
    }

    pub async fn session_view(&self, session: &Session) -> Result<SessionView, AuthError> {
        let auth = self.touch_authenticated(session).await?;
        let memberships = self.list_current_memberships(auth.principal_id).await?;
        let tenant_ids = memberships
            .iter()
            .map(|membership| membership.tenant_id.to_string())
            .collect::<Vec<_>>();
        let active_tenant_id = auth
            .active_tenant_id
            .filter(|active| memberships.iter().any(|m| m.tenant_id == *active))
            .or_else(|| memberships.first().map(|m| m.tenant_id));

        Ok(SessionView {
            authenticated: true,
            principal: Some(PrincipalRef {
                principal_type: "USER",
                id: auth.principal_id.to_string(),
                issuer: auth.issuer,
            }),
            tenant_ids,
            active_tenant_id: active_tenant_id.map(|id| id.to_string()),
            expires_at: Some(cmp::min(auth.absolute_expires_at, auth.idle_expires_at)),
            step_up_expires_at: auth
                .step_up_expires_at
                .filter(|expiry| *expiry > Utc::now()),
            csrf_token: Some(auth.csrf_token),
        })
    }

    pub async fn current_membership(
        &self,
        principal_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<CurrentMembership, AuthError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        set_local_uuid(&mut tx, "app.tenant_id", tenant_id).await?;
        let membership = sqlx::query_as::<_, CurrentMembership>(
            r#"
            SELECT m.tenant_id, m.membership_id, m.role,
                   COALESCE(
                     (SELECT array_agg(mr.role ORDER BY mr.role)
                        FROM membership_roles mr
                       WHERE mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id),
                     ARRAY[m.role]
                   ) AS roles,
                   m.membership_epoch, m.permission_epoch
            FROM memberships m
            JOIN tenants t ON t.id = m.tenant_id
            WHERE m.tenant_id = $1
              AND m.principal_id = $2
              AND m.status = 'ACTIVE'
              AND t.status = 'ACTIVE'
            "#,
        )
        .bind(tenant_id)
        .bind(principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?
        .ok_or(AuthError::MembershipDenied)?;
        tx.commit()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(membership)
    }

    /// Admit `membership.accept` pre-body: prove the route tenant exists and is
    /// ACTIVE without requiring any membership. The signed invite binds the
    /// principal later, inside the bootstrap transaction. A foreign or absent
    /// tenant is masked as `NotFound` by the HTTP layer.
    pub async fn tenant_admission(&self, tenant_id: Uuid) -> Result<(), AuthError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        set_local_uuid(&mut tx, "app.tenant_id", tenant_id).await?;
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM tenants WHERE id=$1 AND status='ACTIVE')",
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        if exists {
            Ok(())
        } else {
            Err(AuthError::TenantContextMismatch)
        }
    }

    pub async fn current_authority(
        &self,
        principal_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<CurrentAuthority, AuthError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        set_local_uuid(&mut tx, "app.tenant_id", tenant_id).await?;

        let membership = sqlx::query_as::<_, CurrentMembership>(
            r#"
            SELECT m.tenant_id, m.membership_id, m.role,
                   COALESCE(
                     (SELECT array_agg(mr.role ORDER BY mr.role)
                        FROM membership_roles mr
                       WHERE mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id),
                     ARRAY[m.role]
                   ) AS roles,
                   m.membership_epoch, m.permission_epoch
            FROM memberships m
            JOIN tenants t ON t.id = m.tenant_id
            WHERE m.tenant_id = $1
              AND m.principal_id = $2
              AND m.status = 'ACTIVE'
              AND t.status = 'ACTIVE'
            "#,
        )
        .bind(tenant_id)
        .bind(principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?
        .ok_or(AuthError::MembershipDenied)?;

        let policy = sqlx::query_as::<_, CurrentPolicySource>(
            r#"
            SELECT policy_version, policy_epoch, cedar_source
            FROM authorization_policies
            WHERE tenant_id = $1 AND is_current
            "#,
        )
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?
        .ok_or(AuthError::PolicyUnavailable)?;

        tx.commit()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(CurrentAuthority { membership, policy })
    }

    pub async fn list_current_memberships(
        &self,
        principal_id: Uuid,
    ) -> Result<Vec<CurrentMembership>, AuthError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        set_local_uuid(&mut tx, "app.principal_id", principal_id).await?;
        let memberships = sqlx::query_as::<_, CurrentMembership>(
            r#"
            SELECT m.tenant_id, m.membership_id, m.role,
                   COALESCE(
                     (SELECT array_agg(mr.role ORDER BY mr.role)
                        FROM membership_roles mr
                       WHERE mr.tenant_id=m.tenant_id AND mr.membership_id=m.membership_id),
                     ARRAY[m.role]
                   ) AS roles,
                   m.membership_epoch, m.permission_epoch
            FROM memberships m
            JOIN tenants t ON t.id = m.tenant_id
            WHERE m.principal_id = $1
              AND m.status = 'ACTIVE'
              AND t.status = 'ACTIVE'
            ORDER BY m.tenant_id
            "#,
        )
        .bind(principal_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?;
        tx.commit()
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?;
        Ok(memberships)
    }

    async fn read_envelope(&self, session: &Session) -> Result<SessionEnvelope, AuthError> {
        session
            .get::<SessionEnvelope>(AUTH_SESSION_KEY)
            .await
            .map_err(|_| AuthError::DependencyUnavailable)?
            .ok_or(AuthError::Unauthenticated)
    }

    fn ensure_not_expired(
        &self,
        envelope: &SessionEnvelope,
        now: DateTime<Utc>,
    ) -> Result<(), AuthError> {
        if now >= envelope.absolute_expires_at || now >= envelope.last_seen_at + self.idle_timeout {
            return Err(AuthError::SessionExpired);
        }
        Ok(())
    }

    fn project(&self, envelope: &SessionEnvelope) -> AuthenticatedSession {
        AuthenticatedSession {
            principal_id: envelope.principal_id,
            issuer: envelope.issuer.clone(),
            subject: envelope.subject.clone(),
            absolute_expires_at: envelope.absolute_expires_at,
            idle_expires_at: envelope.last_seen_at + self.idle_timeout,
            step_up_expires_at: envelope.step_up_expires_at,
            csrf_token: envelope.csrf_token.clone(),
            active_tenant_id: envelope.active_tenant_id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CsrfGuard {
    expected_origin: String,
}

impl CsrfGuard {
    pub fn new(expected_origin: impl Into<String>) -> Result<Self, AuthError> {
        let expected_origin = expected_origin.into();
        let uri = expected_origin
            .parse::<Uri>()
            .map_err(|_| AuthError::DependencyUnavailable)?;
        let scheme = uri.scheme_str().ok_or(AuthError::DependencyUnavailable)?;
        let host = uri.host().ok_or(AuthError::DependencyUnavailable)?;
        let allowed_transport = scheme == "https"
            || (scheme == "http" && matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
        if expected_origin.is_empty()
            || expected_origin.ends_with('/')
            || !allowed_transport
            || uri.query().is_some()
            || uri.path() != "/"
        {
            return Err(AuthError::DependencyUnavailable);
        }
        Ok(Self { expected_origin })
    }

    pub fn verify(
        &self,
        headers: &HeaderMap,
        auth: &AuthenticatedSession,
    ) -> Result<(), AuthError> {
        let origin = headers
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .ok_or(AuthError::CsrfRejected)?;
        let csrf = headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or(AuthError::CsrfRejected)?;
        if origin != self.expected_origin || csrf != auth.csrf_token {
            return Err(AuthError::CsrfRejected);
        }
        Ok(())
    }
}

async fn set_local_uuid(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    value: Uuid,
) -> Result<(), AuthError> {
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(key)
        .bind(value.to_string())
        .execute(&mut **tx)
        .await
        .map_err(|_| AuthError::DependencyUnavailable)?;
    Ok(())
}

fn random_session_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_rejects_missing_origin_and_token() {
        let guard = CsrfGuard::new("https://app.example.test").unwrap();
        let auth = AuthenticatedSession {
            principal_id: Uuid::nil(),
            issuer: "https://id.example.test".into(),
            subject: "sub".into(),
            absolute_expires_at: Utc::now() + ChronoDuration::hours(1),
            idle_expires_at: Utc::now() + ChronoDuration::minutes(30),
            step_up_expires_at: None,
            csrf_token: "token".into(),
            active_tenant_id: None,
        };
        assert_eq!(
            guard.verify(&HeaderMap::new(), &auth),
            Err(AuthError::CsrfRejected)
        );
    }

    #[test]
    fn csrf_requires_exact_configured_origin_and_synchronizer_token() {
        let guard = CsrfGuard::new("https://app.example.test").unwrap();
        let auth = AuthenticatedSession {
            principal_id: Uuid::nil(),
            issuer: "https://id.example.test".into(),
            subject: "sub".into(),
            absolute_expires_at: Utc::now() + ChronoDuration::hours(1),
            idle_expires_at: Utc::now() + ChronoDuration::minutes(30),
            step_up_expires_at: None,
            csrf_token: "token".into(),
            active_tenant_id: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, "https://app.example.test".parse().unwrap());
        headers.insert(CSRF_HEADER, "token".parse().unwrap());
        assert!(guard.verify(&headers, &auth).is_ok());
        headers.insert(ORIGIN, "https://evil.example".parse().unwrap());
        assert_eq!(guard.verify(&headers, &auth), Err(AuthError::CsrfRejected));
    }

    #[test]
    fn secure_cookie_profile_is_the_default() {
        assert_eq!(
            SessionCookieProfile::default(),
            SessionCookieProfile::Secure
        );
    }
}
