use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, PortError, SecretPort};

#[derive(Debug, Default)]
pub struct SecretBrokerAdapter;

impl AdapterBoundary for SecretBrokerAdapter {
    fn adapter_name(&self) -> &'static str {
        "secrets"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl SecretPort for SecretBrokerAdapter {
    fn resolve_for_connector(&self, _reference: &str) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
