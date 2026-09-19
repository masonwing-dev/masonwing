use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, DataPort, PortError};

mod administration;
mod approval_commands;
pub mod artifact_commands;
mod authority;
mod budget_ledger;
mod commands;
mod delegation;
mod effect_commands;
mod identity_commands;
mod invocation;
pub mod migrations;
mod queries;
mod registry;
pub mod registry_validation;
pub mod relay;
mod run_commands;
pub mod store;
pub use identity_commands::{
    InviteTokenKey, MembershipAcceptBootstrapCommand, OneTimeInviteSecret,
    VerifiedInviteAcceptIdentity,
};
pub use relay::{DispatchableRun, OutboxDispatchItem};
pub use run_commands::{CheckpointWrite, DurableCheckpoint};

pub use store::PostgresStore;

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
