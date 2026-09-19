//! Typed values used at the application boundary. Wire requests are additionally
//! checked against the immutable JSON Schema before constructing these values.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Action, ArtifactId, Digest, GrantId, PluginId, PrincipalId, ResourceId, TenantId};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub resource_type: String,
    pub resource_id: ResourceId,
    pub version: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Classification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "PUBLIC",
            Self::Internal => "INTERNAL",
            Self::Confidential => "CONFIDENTIAL",
            Self::Restricted => "RESTRICTED",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: ArtifactId,
    pub tenant_id: TenantId,
    pub digest: Digest,
    pub schema_version: String,
    pub classification: Classification,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRef {
    #[serde(rename = "type")]
    pub kind: PrincipalKind,
    pub id: PrincipalId,
    pub issuer: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrincipalKind {
    User,
    Service,
    Automation,
    Support,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDependency {
    pub plugin_id: PluginId,
    pub contract_range: String,
    pub optional: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UiContribution {
    pub id: String,
    pub slot: String,
    pub path: String,
    pub ui_contract_version: String,
    pub trust: String,
    pub required_action: Action,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginMigration {
    pub id: String,
    pub digest: Digest,
    pub compatibility: String,
    pub reversible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandlerContract {
    pub id: String,
    pub input_schema_ref: ArtifactRef,
    pub output_schema_ref: ArtifactRef,
    pub effects: String,
    pub required_capabilities: Vec<Action>,
    pub max_execution_seconds: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolDescriptor {
    pub id: String,
    pub version: String,
    pub input_schema_ref: ArtifactRef,
    pub output_schema_ref: ArtifactRef,
    pub action: Action,
    pub effect_class: String,
    pub connection_provider: Option<String>,
    pub trust: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionClass {
    TrustedNative,
    WasmComponent,
    RemoteWorker,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: PluginId,
    pub version: String,
    pub contract_version: String,
    pub artifact_digest: Digest,
    pub publisher_id: String,
    pub execution_class: ExecutionClass,
    pub requested_capabilities: Vec<Action>,
    pub dependencies: Vec<PluginDependency>,
    pub ui: Vec<UiContribution>,
    pub migrations: Vec<PluginMigration>,
    pub license_expression: String,
    pub sbom_digest: Digest,
    pub signature_ref: String,
    pub handlers: Vec<HandlerContract>,
    pub workflows: Vec<ArtifactRef>,
    pub data_contracts: Vec<ArtifactRef>,
    pub tools: Vec<ToolDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: String,
    pub handler_id: Option<String>,
    pub input_schema_ref: ArtifactRef,
    pub output_schema_ref: ArtifactRef,
    pub max_attempts: u32,
    pub timeout_seconds: u32,
    pub max_iterations: u32,
    pub on_failure: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    pub condition: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub id: String,
    pub version: String,
    pub input_schema_ref: ArtifactRef,
    pub output_schema_ref: ArtifactRef,
    pub entry_node: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub max_total_steps: u32,
    pub max_model_turns: u32,
    pub required_actions: Vec<Action>,
    pub digest: Digest,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandReceipt {
    pub command_id: ResourceId,
    pub state: ReceiptState,
    pub resource: Option<ResourceRef>,
    pub run_id: Option<ResourceId>,
    pub effect_id: Option<ResourceId>,
    pub correlation_id: ResourceId,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiptState {
    Accepted,
    Succeeded,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: ResourceId,
    pub tenant_id: TenantId,
    pub workflow_id: String,
    pub workflow_version: String,
    pub state: String,
    pub plugin_digest: Digest,
    pub grant_id: GrantId,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub grant_id: GrantId,
    pub tenant_id: TenantId,
    pub principal: PrincipalRef,
    pub actions: Vec<Action>,
    pub resources: Vec<ResourceRef>,
    pub expires_at: DateTime<Utc>,
    pub parent_grant_id: Option<GrantId>,
    pub state: String,
    pub version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceProjection {
    pub resource: ResourceRef,
    pub artifact_ref: ArtifactRef,
    pub updated_at: DateTime<Utc>,
    pub data_state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReadCollection {
    pub items: Vec<ResourceRef>,
    pub next_cursor: Option<String>,
    pub snapshot_at: DateTime<Utc>,
    pub data_state: String,
    pub total_visible: Option<u64>,
}
