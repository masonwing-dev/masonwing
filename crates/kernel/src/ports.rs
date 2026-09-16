//! Typed application ports. Infrastructure crates implement these boundaries;
//! domain plugins never receive raw provider credentials or direct mutation APIs.

use masonwing_contracts::{
    Action, AdapterQualification, ArtifactId, ConnectionHandle, Digest, EffectId, PluginId,
    PrincipalId, ResourceId, TenantId,
};
use thiserror::Error;

pub trait AdapterBoundary: Send + Sync {
    fn adapter_name(&self) -> &'static str;
    fn qualification(&self) -> AdapterQualification;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionRecipe {
    pub product_id: String,
    pub enabled_plugins: Vec<PluginId>,
}

pub trait CompositionPort: AdapterBoundary {
    fn validate_recipe(&self, recipe: &CompositionRecipe) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedIdentity {
    pub principal_id: PrincipalId,
    pub issuer: String,
    pub subject: String,
}

pub trait IdentityPort: AdapterBoundary {
    fn verify_session_reference(
        &self,
        session_reference: &str,
    ) -> Result<VerifiedIdentity, PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipMutation {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub requested_role: String,
}

pub trait MembershipPort: AdapterBoundary {
    fn apply_authorized_membership(&self, mutation: &MembershipMutation) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub action: Action,
    pub resource_id: ResourceId,
}

pub trait AuthorizationPort: AdapterBoundary {
    fn authorize_current(&self, request: &AuthorizationRequest) -> Result<(), PortError>;
}

pub trait DataPort: AdapterBoundary {
    fn write_ready(&self) -> Result<bool, PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRecord {
    pub tenant_id: TenantId,
    pub action: Action,
    pub resource_id: ResourceId,
    pub outcome: String,
}

pub trait AuditPort: AdapterBoundary {
    fn append_in_same_transaction(&self, record: &AuditRecord) -> Result<(), PortError>;
}

pub trait ArtifactPort: AdapterBoundary {
    fn metadata_digest(&self, artifact_id: &ArtifactId) -> Result<Digest, PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub tenant_id: TenantId,
    pub logical_run_id: ResourceId,
    pub workflow_contract: String,
}

pub trait DurableWorkflowPort: AdapterBoundary {
    fn dispatch(&self, request: &WorkflowDispatch) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledOccurrence {
    pub tenant_id: TenantId,
    pub schedule_id: ResourceId,
    pub due_at_epoch_millis: i64,
}

pub trait SchedulerPort: AdapterBoundary {
    fn create_durable_schedule(&self, occurrence: &ScheduledOccurrence) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentInvocation {
    pub tenant_id: TenantId,
    pub plugin_id: PluginId,
    pub operation: String,
}

pub trait ComponentExecutionPort: AdapterBoundary {
    fn invoke_isolated(&self, invocation: &ComponentInvocation) -> Result<(), PortError>;
}

pub trait ModelPort: AdapterBoundary {
    fn supports_capability(&self, capability: &str) -> Result<bool, PortError>;
}

pub trait ConnectionPort: AdapterBoundary {
    fn validate_handle(&self, handle: &ConnectionHandle, action: &Action) -> Result<(), PortError>;
}

pub trait BudgetPort: AdapterBoundary {
    fn reserve_microunits(&self, tenant_id: &TenantId, upper_bound: u64) -> Result<(), PortError>;
}

pub trait EventPort: AdapterBoundary {
    fn enqueue_committed_event(
        &self,
        tenant_id: &TenantId,
        event_id: &str,
    ) -> Result<(), PortError>;
}

pub trait SecretPort: AdapterBoundary {
    fn resolve_for_connector(&self, reference: &str) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiContribution {
    pub plugin_id: PluginId,
    pub contribution_id: String,
    pub schema_digest: Digest,
}

pub trait UiContributionPort: AdapterBoundary {
    fn register_validated(&self, contribution: &UiContribution) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionQuery {
    pub tenant_id: TenantId,
    pub resource_type: String,
    pub cursor: Option<String>,
}

pub trait ProjectionQueryPort: AdapterBoundary {
    fn query_authorized_projection(&self, query: &ProjectionQuery) -> Result<(), PortError>;
}

pub trait ExportPort: AdapterBoundary {
    fn create_authorized_export(
        &self,
        tenant_id: &TenantId,
        source: &ResourceId,
    ) -> Result<ArtifactId, PortError>;
}

pub trait NotificationPort: AdapterBoundary {
    fn mark_read(
        &self,
        tenant_id: &TenantId,
        notification_id: &ResourceId,
    ) -> Result<(), PortError>;
}

pub trait SupportAccessPort: AdapterBoundary {
    fn request_time_bound_access(
        &self,
        tenant_id: &TenantId,
        request_id: &ResourceId,
    ) -> Result<(), PortError>;
}

pub trait ReleaseQualificationPort: AdapterBoundary {
    fn qualify_candidate(&self, candidate_digest: &Digest) -> Result<(), PortError>;
}

pub trait CatalogPort: AdapterBoundary {
    fn submit_candidate(&self, plugin_id: &PluginId, digest: &Digest) -> Result<(), PortError>;
}

pub trait ConformancePort: AdapterBoundary {
    fn run_contract_suite(&self, plugin_id: &PluginId) -> Result<(), PortError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalMutationRequest {
    pub tenant_id: TenantId,
    pub effect_id: EffectId,
    pub action: Action,
    pub target: ResourceId,
    pub approved_digest: Digest,
}

pub trait ExternalMutationPort: AdapterBoundary {
    fn transmit_after_current_guards(
        &self,
        request: &ExternalMutationRequest,
    ) -> Result<(), PortError>;
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PortError {
    #[error("ADAPTER_NOT_QUALIFIED:{adapter}")]
    NotQualified { adapter: &'static str },
    #[error("ADAPTER_UNAVAILABLE:{adapter}")]
    Unavailable { adapter: &'static str },
    #[error("AUTHZ_DENIED")]
    AuthorizationDenied,
}
