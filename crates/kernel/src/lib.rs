//! Domain-neutral Masonwing microkernel invariants.
//!
//! Business domains consume `masonwing-sdk`, not this crate. This crate owns
//! host-side safety invariants and typed ports only.

pub mod approval;
pub mod budget;
pub mod effects;
pub mod grants;
pub mod loop_guard;
pub mod ports;
pub mod registry;
pub mod runtime;
pub mod scope;
pub mod state;

pub use approval::{ApprovalBinding, ApprovalError, DispatchBinding};
pub use budget::{BudgetError, CostReservation};
pub use effects::{EffectError, EffectRecord, ReconciliationEvidence};
pub use grants::{ChildGrantRequest, GrantError, RunGrant};
pub use loop_guard::{BoundedLoop, LoopError};
pub use registry::{DependencyGraph, InstallRegistry, RegistryError};
pub use scope::{ScopedResource, TenantScopeError, require_tenant_scope};
pub use state::{
    ApprovalState, CostState, EffectState, MachineState, PluginState, RunState, StateError,
    VersionedState,
};
