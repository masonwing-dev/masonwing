use masonwing_contracts::{Action, Digest, EpochMillis, PrincipalId, ResourceId, TenantId};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalBinding {
    action: Action,
    target: ResourceId,
    content_digest: Digest,
    target_revision: u64,
    tenant_scope: TenantId,
    actor: PrincipalId,
    expires_at: EpochMillis,
}

impl ApprovalBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action: Action,
        target: ResourceId,
        content_digest: Digest,
        target_revision: u64,
        tenant_scope: TenantId,
        actor: PrincipalId,
        expires_at: EpochMillis,
    ) -> Self {
        Self {
            action,
            target,
            content_digest,
            target_revision,
            tenant_scope,
            actor,
            expires_at,
        }
    }

    pub fn validate_dispatch(
        &self,
        dispatch: &DispatchBinding,
        now: EpochMillis,
    ) -> Result<(), ApprovalError> {
        if now >= self.expires_at {
            return Err(ApprovalError::ApprovalExpired);
        }
        if self.action != dispatch.action
            || self.target != dispatch.target
            || self.content_digest != dispatch.content_digest
            || self.target_revision != dispatch.target_revision
            || self.tenant_scope != dispatch.tenant_scope
        {
            return Err(ApprovalError::ApprovalStale);
        }
        Ok(())
    }

    pub fn action(&self) -> &Action {
        &self.action
    }

    pub fn target(&self) -> &ResourceId {
        &self.target
    }

    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    pub fn target_revision(&self) -> u64 {
        self.target_revision
    }

    pub fn tenant_scope(&self) -> &TenantId {
        &self.tenant_scope
    }

    pub fn actor(&self) -> &PrincipalId {
        &self.actor
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchBinding {
    pub action: Action,
    pub target: ResourceId,
    pub content_digest: Digest,
    pub target_revision: u64,
    pub tenant_scope: TenantId,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ApprovalError {
    #[error("APPROVAL_STALE")]
    ApprovalStale,
    #[error("APPROVAL_EXPIRED")]
    ApprovalExpired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> Digest {
        Digest::sha256(byte.to_string().repeat(64)).unwrap()
    }

    fn approval() -> ApprovalBinding {
        ApprovalBinding::new(
            Action::new("publication.update").unwrap(),
            ResourceId::new("target_a").unwrap(),
            digest('a'),
            2,
            TenantId::new("tenant_a").unwrap(),
            PrincipalId::new("reviewer_a").unwrap(),
            EpochMillis(1_000),
        )
    }

    fn dispatch(content_digest: Digest) -> DispatchBinding {
        DispatchBinding {
            action: Action::new("publication.update").unwrap(),
            target: ResourceId::new("target_a").unwrap(),
            content_digest,
            target_revision: 2,
            tenant_scope: TenantId::new("tenant_a").unwrap(),
        }
    }

    // MASONWING@1.0.1 REQ-078 / AC-082 / TC-AC-082.
    #[test]
    fn approval_record_keeps_exact_immutable_binding() {
        let approval = approval();
        assert_eq!(approval.content_digest(), &digest('a'));
        assert_eq!(approval.target().as_str(), "target_a");
        assert_eq!(approval.target_revision(), 2);
        assert_eq!(approval.actor().as_str(), "reviewer_a");
        assert_eq!(approval.tenant_scope().as_str(), "tenant_a");
        assert_eq!(approval.action().as_str(), "publication.update");
    }

    // MASONWING@1.0.1 REQ-079 / AC-083 / TC-AC-083.
    #[test]
    fn changed_content_digest_invalidates_approval() {
        assert_eq!(
            approval().validate_dispatch(&dispatch(digest('b')), EpochMillis(500)),
            Err(ApprovalError::ApprovalStale)
        );
    }

    // MASONWING@1.0.1 REQ-080 / AC-084 / TC-AC-084.
    #[test]
    fn approval_expires_at_exact_boundary() {
        assert_eq!(
            approval().validate_dispatch(&dispatch(digest('a')), EpochMillis(1_000)),
            Err(ApprovalError::ApprovalExpired)
        );
    }
}
