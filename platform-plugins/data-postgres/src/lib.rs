use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, DataPort, PortError};

#[derive(Debug, Default)]
pub struct PostgresDataAdapter;

impl AdapterBoundary for PostgresDataAdapter {
    fn adapter_name(&self) -> &'static str {
        "data-postgres"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl DataPort for PostgresDataAdapter {
    fn write_ready(&self) -> Result<bool, PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
