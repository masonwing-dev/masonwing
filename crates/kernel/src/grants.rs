use std::collections::BTreeSet;

use masonwing_contracts::{Action, EpochMillis, GrantId, PrincipalId, ResourceId, TenantId};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunGrant {
    id: GrantId,
    tenant_id: TenantId,
    principal_id: PrincipalId,
    parent_id: Option<GrantId>,
    actions: BTreeSet<Action>,
    resources: BTreeSet<ResourceId>,
    expires_at: EpochMillis,
}

impl RunGrant {
    pub fn new_root(
        id: GrantId,
        tenant_id: TenantId,
        principal_id: PrincipalId,
        actions: impl IntoIterator<Item = Action>,
        resources: impl IntoIterator<Item = ResourceId>,
        expires_at: EpochMillis,
    ) -> Self {
        Self {
            id,
            tenant_id,
            principal_id,
            parent_id: None,
            actions: actions.into_iter().collect(),
            resources: resources.into_iter().collect(),
            expires_at,
        }
    }

    pub fn derive_child(&self, request: ChildGrantRequest) -> Result<Self, GrantError> {
        if request.tenant_id != self.tenant_id
            || request.expires_at > self.expires_at
            || !request.actions.is_subset(&self.actions)
            || !request.resources.is_subset(&self.resources)
        {
            return Err(GrantError::DelegationDenied);
        }

        Ok(Self {
            id: request.id,
            tenant_id: request.tenant_id,
            principal_id: request.principal_id,
            parent_id: Some(self.id.clone()),
            actions: request.actions,
            resources: request.resources,
            expires_at: request.expires_at,
        })
    }

    pub fn authorize(
        &self,
        action: &Action,
        resource: &ResourceId,
        now: EpochMillis,
    ) -> Result<(), GrantError> {
        if now >= self.expires_at {
            return Err(GrantError::DelegationExpired);
        }
        if !self.actions.contains(action) || !self.resources.contains(resource) {
            return Err(GrantError::DelegationDenied);
        }
        Ok(())
    }

    pub fn parent_id(&self) -> Option<&GrantId> {
        self.parent_id.as_ref()
    }

    pub fn actions(&self) -> &BTreeSet<Action> {
        &self.actions
    }

    pub fn resources(&self) -> &BTreeSet<ResourceId> {
        &self.resources
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildGrantRequest {
    pub id: GrantId,
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub actions: BTreeSet<Action>,
    pub resources: BTreeSet<ResourceId>,
    pub expires_at: EpochMillis,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum GrantError {
    #[error("DELEGATION_DENIED")]
    DelegationDenied,
    #[error("DELEGATION_EXPIRED")]
    DelegationExpired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(value: &str) -> Action {
        Action::new(value).unwrap()
    }

    fn resource(value: &str) -> ResourceId {
        ResourceId::new(value).unwrap()
    }

    fn parent() -> RunGrant {
        RunGrant::new_root(
            GrantId::new("grant_parent").unwrap(),
            TenantId::new("tenant_a").unwrap(),
            PrincipalId::new("principal_a").unwrap(),
            [action("article.read")],
            [resource("article_a")],
            EpochMillis(1_000),
        )
    }

    // MASONWING@1.0.1 REQ-050 / AC-052 / TC-AC-052.
    #[test]
    fn wider_plugin_capability_does_not_expand_run_grant() {
        assert_eq!(
            parent().authorize(
                &action("article.publish"),
                &resource("article_a"),
                EpochMillis(5)
            ),
            Err(GrantError::DelegationDenied)
        );
    }

    // MASONWING@1.0.1 REQ-052 / AC-054 / TC-AC-054.
    #[test]
    fn grant_is_expired_at_exact_expiry_instant() {
        assert_eq!(
            parent().authorize(
                &action("article.read"),
                &resource("article_a"),
                EpochMillis(1_000)
            ),
            Err(GrantError::DelegationExpired)
        );
    }

    // MASONWING@1.0.1 REQ-054 / AC-056 / TC-AC-056.
    #[test]
    fn child_grant_cannot_expand_parent_resource_set() {
        let request = ChildGrantRequest {
            id: GrantId::new("grant_child").unwrap(),
            tenant_id: TenantId::new("tenant_a").unwrap(),
            principal_id: PrincipalId::new("principal_child").unwrap(),
            actions: [action("article.read")].into_iter().collect(),
            resources: [resource("article_a"), resource("article_b")]
                .into_iter()
                .collect(),
            expires_at: EpochMillis(900),
        };

        assert_eq!(
            parent().derive_child(request),
            Err(GrantError::DelegationDenied)
        );
    }
}
