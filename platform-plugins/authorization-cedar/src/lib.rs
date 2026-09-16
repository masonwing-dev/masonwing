use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{
    AdapterBoundary, AuthorizationPort, AuthorizationRequest, PortError,
};

#[derive(Debug, Default)]
pub struct CedarAuthorizationAdapter;

impl AdapterBoundary for CedarAuthorizationAdapter {
    fn adapter_name(&self) -> &'static str {
        "authorization-cedar"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl AuthorizationPort for CedarAuthorizationAdapter {
    fn authorize_current(&self, _request: &AuthorizationRequest) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
