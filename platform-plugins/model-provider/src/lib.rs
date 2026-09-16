use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, ModelPort, PortError};

#[derive(Debug, Default)]
pub struct ModelProviderAdapter;

impl AdapterBoundary for ModelProviderAdapter {
    fn adapter_name(&self) -> &'static str {
        "model-provider"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl ModelPort for ModelProviderAdapter {
    fn supports_capability(&self, _capability: &str) -> Result<bool, PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
