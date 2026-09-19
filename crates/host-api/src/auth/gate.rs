use chrono::{DateTime, Utc};
use tower_sessions::Session;
use uuid::Uuid;

use masonwing_authorization_cedar::{
    CedarAuthorizationAdapter, CedarPolicySnapshot, TrustedAuthorizationRequest, TrustedPrincipal,
    TrustedResource,
};

use super::{
    error::AuthError,
    session::{CurrentMembership, SessionAuthority},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedActor {
    pub tenant_id: Uuid,
    pub principal_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub membership_id: Uuid,
    pub role: String,
    pub roles: Vec<String>,
    pub membership_epoch: i64,
    pub permission_epoch: i64,
    pub policy_version: String,
    pub policy_epoch: i64,
    pub step_up_expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthorizationInput {
    pub action: String,
    /// This resource must be constructed from host/database facts. Do not
    /// deserialize this struct directly from an untrusted plugin/client body.
    pub resource: TrustedResource,
    pub requires_step_up: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentPermit {
    pub actor: VerifiedActor,
    pub policy_version: String,
    pub policy_epoch: i64,
}

#[derive(Clone)]
pub struct AuthGate {
    sessions: SessionAuthority,
    cedar: CedarAuthorizationAdapter,
}

impl AuthGate {
    pub fn new(sessions: SessionAuthority, cedar: CedarAuthorizationAdapter) -> Self {
        Self { sessions, cedar }
    }

    /// Resolve the browser identity and re-read current ACTIVE membership from
    /// PostgreSQL. Route-derived tenant_id is authoritative; no tenant body/header
    /// value is accepted by this method.
    pub async fn verify_for_tenant(
        &self,
        session: &Session,
        tenant_id: Uuid,
    ) -> Result<VerifiedActor, AuthError> {
        let auth = self.sessions.touch_authenticated(session).await?;
        let current = self
            .sessions
            .current_authority(auth.principal_id, tenant_id)
            .await?;
        Ok(project_actor(
            &auth,
            current.membership,
            &current.policy.policy_version,
            current.policy.policy_epoch,
        ))
    }

    /// Resolve current membership and current immutable Cedar policy version,
    /// then authorize using only host-built entity facts.
    pub async fn authorize_current(
        &self,
        session: &Session,
        tenant_id: Uuid,
        input: AuthorizationInput,
    ) -> Result<CurrentPermit, AuthError> {
        self.authorize_current_and_nested(session, tenant_id, input, &[])
            .await
    }

    /// Authorize an outer command and all requested nested capabilities against
    /// one current membership/policy snapshot. This is the safe gate for
    /// `grant.create`, `effect.propose` and similar operations: possessing the
    /// outer action never implies possession of a broader delegated action.
    pub async fn authorize_current_and_nested(
        &self,
        session: &Session,
        tenant_id: Uuid,
        outer: AuthorizationInput,
        nested: &[AuthorizationInput],
    ) -> Result<CurrentPermit, AuthError> {
        let auth = self.sessions.touch_authenticated(session).await?;
        let current = self
            .sessions
            .current_authority(auth.principal_id, tenant_id)
            .await?;
        let snapshot = CedarPolicySnapshot::parse(
            current.policy.policy_version.clone(),
            current.policy.policy_epoch,
            &current.policy.cedar_source,
        )?;
        let actor = project_actor(
            &auth,
            current.membership,
            &current.policy.policy_version,
            current.policy.policy_epoch,
        );

        self.evaluate_inputs(&auth, &actor, &snapshot, tenant_id, outer, nested)?;

        Ok(CurrentPermit {
            policy_version: actor.policy_version.clone(),
            policy_epoch: actor.policy_epoch,
            actor,
        })
    }

    fn evaluate_inputs(
        &self,
        auth: &super::session::AuthenticatedSession,
        actor: &VerifiedActor,
        snapshot: &CedarPolicySnapshot,
        tenant_id: Uuid,
        outer: AuthorizationInput,
        nested: &[AuthorizationInput],
    ) -> Result<(), AuthError> {
        self.evaluate_one(auth, actor, snapshot, tenant_id, outer)?;
        for nested_input in nested.iter().cloned() {
            self.evaluate_one(auth, actor, snapshot, tenant_id, nested_input)?;
        }
        Ok(())
    }

    fn evaluate_one(
        &self,
        auth: &super::session::AuthenticatedSession,
        actor: &VerifiedActor,
        snapshot: &CedarPolicySnapshot,
        tenant_id: Uuid,
        input: AuthorizationInput,
    ) -> Result<(), AuthError> {
        if input.resource.tenant_id != tenant_id.to_string() {
            // Resource tenant is an internal host fact. Returning the generic
            // not-found shape avoids confirming a cross-tenant resource.
            return Err(AuthError::NotFound);
        }
        if input.requires_step_up
            && auth
                .step_up_expires_at
                .is_none_or(|expires_at| expires_at <= Utc::now())
        {
            return Err(AuthError::StepUpRequired);
        }

        let cedar_request = TrustedAuthorizationRequest {
            principal: TrustedPrincipal {
                principal_id: actor.principal_id.to_string(),
                tenant_id: actor.tenant_id.to_string(),
                role: actor.role.clone(),
                roles: actor.roles.clone(),
                membership_epoch: actor.membership_epoch,
                permission_epoch: actor.permission_epoch,
            },
            action: input.action,
            resource: input.resource,
            context: Default::default(),
        };
        self.cedar.evaluate(snapshot, &cedar_request)?;
        Ok(())
    }
}

fn project_actor(
    auth: &super::session::AuthenticatedSession,
    membership: CurrentMembership,
    policy_version: &str,
    policy_epoch: i64,
) -> VerifiedActor {
    VerifiedActor {
        tenant_id: membership.tenant_id,
        principal_id: auth.principal_id,
        issuer: auth.issuer.clone(),
        subject: auth.subject.clone(),
        membership_id: membership.membership_id,
        role: membership.role,
        roles: membership.roles,
        membership_epoch: membership.membership_epoch,
        permission_epoch: membership.permission_epoch,
        policy_version: policy_version.to_owned(),
        policy_epoch,
        step_up_expires_at: auth.step_up_expires_at,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::auth::session::AuthenticatedSession;

    fn auth_and_actor(tenant_id: Uuid) -> (AuthenticatedSession, VerifiedActor) {
        let principal_id = Uuid::new_v4();
        let auth = AuthenticatedSession {
            principal_id,
            issuer: "https://id.example.test/realms/masonwing".into(),
            subject: "subject-1".into(),
            absolute_expires_at: Utc::now() + Duration::hours(1),
            idle_expires_at: Utc::now() + Duration::minutes(30),
            step_up_expires_at: None,
            csrf_token: "csrf".into(),
            active_tenant_id: Some(tenant_id),
        };
        let actor = VerifiedActor {
            tenant_id,
            principal_id,
            issuer: auth.issuer.clone(),
            subject: auth.subject.clone(),
            membership_id: Uuid::new_v4(),
            role: "EDITOR".into(),
            roles: vec!["EDITOR".into()],
            membership_epoch: 4,
            permission_epoch: 9,
            policy_version: "1.2.3".into(),
            policy_epoch: 12,
            step_up_expires_at: None,
        };
        (auth, actor)
    }

    fn input(tenant_id: Uuid, action: &str) -> AuthorizationInput {
        AuthorizationInput {
            action: action.into(),
            resource: TrustedResource::new("Document", "doc-1", tenant_id.to_string()),
            requires_step_up: false,
        }
    }

    #[tokio::test]
    async fn nested_action_must_be_independently_permitted() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let gate = AuthGate::new(SessionAuthority::new(pool), CedarAuthorizationAdapter);
        let tenant_id = Uuid::new_v4();
        let (auth, actor) = auth_and_actor(tenant_id);
        let snapshot = CedarPolicySnapshot::parse(
            "1.2.3",
            12,
            r#"permit(principal, action == Action::"grant.create", resource);"#,
        )
        .unwrap();

        let result = gate.evaluate_inputs(
            &auth,
            &actor,
            &snapshot,
            tenant_id,
            input(tenant_id, "grant.create"),
            &[input(tenant_id, "effect.dispatch")],
        );

        assert_eq!(result, Err(AuthError::AuthorizationDenied));
    }

    #[tokio::test]
    async fn nested_checks_share_actor_policy_epoch_binding() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let gate = AuthGate::new(SessionAuthority::new(pool), CedarAuthorizationAdapter);
        let tenant_id = Uuid::new_v4();
        let (auth, actor) = auth_and_actor(tenant_id);
        let snapshot = CedarPolicySnapshot::parse(
            actor.policy_version.clone(),
            actor.policy_epoch,
            r#"
                permit(principal, action == Action::"grant.create", resource);
                permit(principal, action == Action::"document.read", resource);
            "#,
        )
        .unwrap();

        assert!(
            gate.evaluate_inputs(
                &auth,
                &actor,
                &snapshot,
                tenant_id,
                input(tenant_id, "grant.create"),
                &[input(tenant_id, "document.read")],
            )
            .is_ok()
        );
        assert_eq!(actor.policy_version, snapshot.policy_version);
        assert_eq!(actor.policy_epoch, snapshot.policy_epoch);
    }
}
