use masonwing_contracts::{ResourceId, TenantId};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedResource {
    tenant_id: TenantId,
    resource_id: ResourceId,
}

impl ScopedResource {
    pub fn new(tenant_id: TenantId, resource_id: ResourceId) -> Self {
        Self {
            tenant_id,
            resource_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }
}

pub fn require_tenant_scope(
    authenticated_tenant: &TenantId,
    resource: &ScopedResource,
) -> Result<(), TenantScopeError> {
    if authenticated_tenant != resource.tenant_id() {
        return Err(TenantScopeError::TenantContextMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum TenantScopeError {
    #[error("TENANT_CONTEXT_MISMATCH")]
    TenantContextMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-047 / AC-049.
    #[test]
    fn authenticated_tenant_wins_over_conflicting_resource_scope() {
        let tenant_a = TenantId::new("tenant_a").unwrap();
        let tenant_b = TenantId::new("tenant_b").unwrap();
        let resource = ScopedResource::new(tenant_b, ResourceId::new("resource_b").unwrap());

        assert_eq!(
            require_tenant_scope(&tenant_a, &resource),
            Err(TenantScopeError::TenantContextMismatch)
        );
    }
}
