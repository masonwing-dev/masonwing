use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, DurableWorkflowPort, PortError, WorkflowDispatch};

#[derive(Debug, Default)]
pub struct TemporalWorkflowAdapter;

impl AdapterBoundary for TemporalWorkflowAdapter {
    fn adapter_name(&self) -> &'static str {
        "workflow-temporal"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl DurableWorkflowPort for TemporalWorkflowAdapter {
    fn dispatch(&self, _request: &WorkflowDispatch) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
