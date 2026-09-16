use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, IdentityPort, PortError, VerifiedIdentity};

#[derive(Debug, Default)]
pub struct OidcIdentityAdapter;

impl AdapterBoundary for OidcIdentityAdapter {
    fn adapter_name(&self) -> &'static str {
        "identity-oidc"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl IdentityPort for OidcIdentityAdapter {
    fn verify_session_reference(
        &self,
        _session_reference: &str,
    ) -> Result<VerifiedIdentity, PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
