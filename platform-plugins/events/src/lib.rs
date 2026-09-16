use masonwing_contracts::{AdapterQualification, TenantId};
use masonwing_kernel::ports::{AdapterBoundary, EventPort, PortError};

#[derive(Debug, Default)]
pub struct EventBusAdapter;

impl AdapterBoundary for EventBusAdapter {
    fn adapter_name(&self) -> &'static str {
        "events"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl EventPort for EventBusAdapter {
    fn enqueue_committed_event(
        &self,
        _tenant_id: &TenantId,
        _event_id: &str,
    ) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
