use masonwing_contracts::{Action, AdapterQualification, ConnectionHandle};
use masonwing_kernel::ports::{
    AdapterBoundary, ConnectionPort, ExternalMutationPort, ExternalMutationRequest, PortError,
};

#[derive(Debug, Default)]
pub struct ConnectionBrokerAdapter;

impl AdapterBoundary for ConnectionBrokerAdapter {
    fn adapter_name(&self) -> &'static str {
        "connections"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl ConnectionPort for ConnectionBrokerAdapter {
    fn validate_handle(
        &self,
        _handle: &ConnectionHandle,
        _action: &Action,
    ) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}

impl ExternalMutationPort for ConnectionBrokerAdapter {
    fn transmit_after_current_guards(
        &self,
        _request: &ExternalMutationRequest,
    ) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
