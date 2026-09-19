//! Public SDK surface for Masonwing domain plugins.
//!
//! The SDK deliberately exposes effect *proposal* capability, never a direct
//! provider mutation primitive. The host remains the authority and effect broker.

use std::collections::BTreeSet;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub use masonwing_contracts::wire::{ArtifactRef, Classification, ExecutionClass, HandlerContract};
pub use masonwing_contracts::*;

pub const SUPPORTED_CONTRACT_RANGE: &str = ">=1.0.0,<2.0.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub id: &'static str,
    pub contract_version: &'static str,
    pub feature_namespaces: &'static [&'static str],
    pub operations: &'static [&'static str],
    pub trace_ids: &'static [&'static str],
}

pub trait DomainPlugin: Send + Sync {
    fn descriptor(&self) -> PluginDescriptor;

    fn invoke(
        &self,
        context: &InvocationContext,
        input: &InvocationInput,
        host: &dyn HostEffectBroker,
    ) -> Result<InvocationResult, SdkError> {
        self.validate_invocation(context, input)?;
        let _ = host;
        let descriptor = self.descriptor();
        Err(SdkError::NotImplemented {
            operation: context.handler_id.clone(),
            trace_ids: descriptor.trace_ids,
        })
    }

    fn validate_invocation(
        &self,
        context: &InvocationContext,
        input: &InvocationInput,
    ) -> Result<(), SdkError> {
        let descriptor = self.descriptor();
        ensure_contract_compatible(descriptor.contract_version)?;
        if descriptor.id != context.plugin_id.as_str() {
            return Err(SdkError::PluginIdentityMismatch);
        }
        if !descriptor.operations.contains(&context.handler_id.as_str()) {
            return Err(SdkError::UnsupportedOperation(context.handler_id.clone()));
        }
        if input.schema_ref != context.input_schema_ref {
            return Err(SdkError::InputSchemaMismatch);
        }
        Ok(())
    }

    fn invoke_scaffold(&self, operation: &str) -> Result<(), SdkError> {
        let descriptor = self.descriptor();
        if !descriptor.operations.contains(&operation) {
            return Err(SdkError::UnsupportedOperation(operation.to_owned()));
        }
        Err(SdkError::NotImplemented {
            operation: operation.to_owned(),
            trace_ids: descriptor.trace_ids,
        })
    }
}

/// Deterministic first-party package material used by operator bootstrap and
/// registry/signing tests. `artifact_bytes` are the immutable package payload;
/// callers must verify `artifact_digest` before trusting the package identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagedPluginFixture {
    pub plugin_id: PluginId,
    pub version: String,
    pub contract_version: String,
    pub publisher_id: String,
    pub execution_class: ExecutionClass,
    pub artifact_bytes: &'static [u8],
    pub artifact_digest: Digest,
    pub handlers: Vec<HandlerContract>,
    pub workflows: Vec<ArtifactRef>,
    /// Immutable schema/workflow payloads required to seed the operator artifact
    /// store before registering the fixture manifest.
    pub supporting_artifacts: Vec<PackagedArtifact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagedArtifact {
    pub reference: ArtifactRef,
    pub bytes: Vec<u8>,
}

impl PackagedArtifact {
    pub fn validate(&self) -> Result<(), SdkError> {
        if sha256_digest(&self.bytes) != self.reference.digest {
            return Err(SdkError::InvalidPackage("supporting_artifact_digest"));
        }
        Ok(())
    }
}

impl PackagedPluginFixture {
    pub fn validate(&self) -> Result<(), SdkError> {
        ensure_contract_compatible(&self.contract_version)?;
        if self.publisher_id.trim().is_empty() || self.version.trim().is_empty() {
            return Err(SdkError::InvalidPackage("metadata"));
        }
        if sha256_digest(self.artifact_bytes) != self.artifact_digest {
            return Err(SdkError::InvalidPackage("artifact_digest"));
        }
        if self.handlers.is_empty() {
            return Err(SdkError::InvalidPackage("handlers"));
        }
        let mut handler_ids = BTreeSet::new();
        for handler in &self.handlers {
            if handler.id.trim().is_empty()
                || !handler_ids.insert(handler.id.as_str())
                || handler.max_execution_seconds == 0
                || handler.input_schema_ref.tenant_id != handler.output_schema_ref.tenant_id
                || !matches!(
                    handler.effects.as_str(),
                    "READ_ONLY" | "PROPOSES_EFFECT" | "DETERMINISTIC"
                )
            {
                return Err(SdkError::InvalidPackage("handler_contract"));
            }
        }
        for artifact in &self.supporting_artifacts {
            artifact.validate()?;
        }
        for workflow in &self.workflows {
            if !self
                .supporting_artifacts
                .iter()
                .any(|artifact| artifact.reference == *workflow)
            {
                return Err(SdkError::InvalidPackage("workflow_payload"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationContext {
    pub invocation_id: ResourceId,
    pub tenant_id: TenantId,
    pub plugin_id: PluginId,
    pub handler_id: String,
    pub grant_id: GrantId,
    pub contract_version: String,
    pub input_schema_ref: ArtifactRef,
    pub output_schema_ref: ArtifactRef,
    granted_capabilities: BTreeSet<Action>,
}

impl InvocationContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        invocation_id: ResourceId,
        tenant_id: TenantId,
        plugin_id: PluginId,
        handler_id: impl Into<String>,
        grant_id: GrantId,
        contract_version: impl Into<String>,
        input_schema_ref: ArtifactRef,
        output_schema_ref: ArtifactRef,
        granted_capabilities: impl IntoIterator<Item = Action>,
    ) -> Result<Self, SdkError> {
        let handler_id = handler_id.into();
        if handler_id.is_empty() || handler_id.len() > 128 {
            return Err(SdkError::InvalidInvocation("handler_id"));
        }
        let contract_version = contract_version.into();
        ensure_contract_compatible(&contract_version)?;
        if input_schema_ref.tenant_id != tenant_id || output_schema_ref.tenant_id != tenant_id {
            return Err(SdkError::InvalidInvocation("schema_tenant"));
        }
        Ok(Self {
            invocation_id,
            tenant_id,
            plugin_id,
            handler_id,
            grant_id,
            contract_version,
            input_schema_ref,
            output_schema_ref,
            granted_capabilities: granted_capabilities.into_iter().collect(),
        })
    }

    pub fn allows(&self, action: &Action) -> bool {
        self.granted_capabilities.contains(action)
    }

    pub fn granted_capabilities(&self) -> impl ExactSizeIterator<Item = &Action> {
        self.granted_capabilities.iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationInput {
    pub schema_ref: ArtifactRef,
    /// Serialized application payload. When `schema_ref` resolves to a JSON
    /// Schema document, these bytes are UTF-8 canonical JSON for the instance
    /// being validated; they are never a schema digest or schema document.
    pub bytes: Vec<u8>,
}

impl InvocationInput {
    pub fn new(schema_ref: ArtifactRef, bytes: Vec<u8>) -> Self {
        Self { schema_ref, bytes }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationOutput {
    pub schema_ref: ArtifactRef,
    /// Serialized application payload using the same representation contract as
    /// `InvocationInput::bytes`.
    pub bytes: Vec<u8>,
}

impl InvocationOutput {
    pub fn for_context(context: &InvocationContext, bytes: Vec<u8>) -> Self {
        Self {
            schema_ref: context.output_schema_ref.clone(),
            bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvocationResult {
    pub output: InvocationOutput,
    pub proposed_effects: Vec<EffectProposalHandle>,
}

impl InvocationResult {
    pub fn pure(context: &InvocationContext, bytes: Vec<u8>) -> Self {
        Self {
            output: InvocationOutput::for_context(context, bytes),
            proposed_effects: Vec::new(),
        }
    }

    pub fn validate_output(&self, context: &InvocationContext) -> Result<(), SdkError> {
        if self.output.schema_ref != context.output_schema_ref {
            return Err(SdkError::OutputSchemaMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectProposal {
    pub tenant_id: TenantId,
    pub action: Action,
    pub target: ResourceId,
    pub content_digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectProposalHandle {
    pub effect_id: EffectId,
}

/// Capability available to domain handlers. It can ask the host to create an
/// effect proposal; there is intentionally no `transmit` method here.
pub trait HostEffectBroker: Send + Sync {
    fn propose_effect(&self, proposal: EffectProposal) -> Result<EffectProposalHandle, SdkError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DenyEffects;

impl HostEffectBroker for DenyEffects {
    fn propose_effect(&self, _proposal: EffectProposal) -> Result<EffectProposalHandle, SdkError> {
        Err(SdkError::CapabilityDenied("effect.propose"))
    }
}

/// Generic public finite-state helper for domain contracts. The SDK owns only
/// transition mechanics; each domain supplies its own declared edge table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionEdge {
    pub from: &'static str,
    pub to: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct FiniteStateContract {
    edges: &'static [TransitionEdge],
}

impl FiniteStateContract {
    pub const fn new(edges: &'static [TransitionEdge]) -> Self {
        Self { edges }
    }

    pub fn transition(
        &self,
        state: &mut VersionedContractState,
        target: &str,
    ) -> Result<(), ContractStateError> {
        if !self
            .edges
            .iter()
            .any(|edge| edge.from == state.state && edge.to == target)
        {
            return Err(ContractStateError::IllegalTransition);
        }
        state.state = target.to_owned();
        state.version += 1;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedContractState {
    state: String,
    version: u64,
}

impl VersionedContractState {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            version: 1,
        }
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ContractStateError {
    #[error("ILLEGAL_TRANSITION")]
    IllegalTransition,
}

pub fn sha256_digest(bytes: &[u8]) -> Digest {
    let raw = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in raw {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    Digest::sha256(hex).expect("SHA-256 encoder always produces lowercase 64-hex")
}

pub fn ensure_contract_compatible(required: &str) -> Result<(), SdkError> {
    let mut parts = required.split('.');
    let major = parts.next().and_then(|part| part.parse::<u64>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u64>().ok());
    let patch = parts.next().and_then(|part| part.parse::<u64>().ok());
    if parts.next().is_some() || minor.is_none() || patch.is_none() {
        return Err(SdkError::ContractIncompatible {
            required: required.to_owned(),
            supported: SUPPORTED_CONTRACT_RANGE,
        });
    }
    if major != Some(1) {
        return Err(SdkError::ContractIncompatible {
            required: required.to_owned(),
            supported: SUPPORTED_CONTRACT_RANGE,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SdkError {
    #[error("NOT_IMPLEMENTED:{operation}")]
    NotImplemented {
        operation: String,
        trace_ids: &'static [&'static str],
    },
    #[error("UNSUPPORTED_OPERATION:{0}")]
    UnsupportedOperation(String),
    #[error("HOST_CAPABILITY_UNAVAILABLE:{0}")]
    HostCapabilityUnavailable(&'static str),
    #[error("CONTRACT_INCOMPATIBLE:required={required};supported={supported}")]
    ContractIncompatible {
        required: String,
        supported: &'static str,
    },
    #[error("PLUGIN_IDENTITY_MISMATCH")]
    PluginIdentityMismatch,
    #[error("INPUT_SCHEMA_MISMATCH")]
    InputSchemaMismatch,
    #[error("OUTPUT_SCHEMA_MISMATCH")]
    OutputSchemaMismatch,
    #[error("CAPABILITY_DENIED:{0}")]
    CapabilityDenied(&'static str),
    #[error("INVALID_PAYLOAD:{0}")]
    InvalidPayload(&'static str),
    #[error("INVALID_INVOCATION:{0}")]
    InvalidInvocation(&'static str),
    #[error("INVALID_PLUGIN_PACKAGE:{0}")]
    InvalidPackage(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_helper_uses_public_contract_format() {
        assert_eq!(
            sha256_digest(b"abc").as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn public_state_contract_preserves_state_and_version_on_illegal_edge() {
        const EDGES: &[TransitionEdge] = &[TransitionEdge { from: "A", to: "B" }];
        let contract = FiniteStateContract::new(EDGES);
        let mut state = VersionedContractState::new("A");

        assert_eq!(
            contract.transition(&mut state, "C"),
            Err(ContractStateError::IllegalTransition)
        );
        assert_eq!(state.state(), "A");
        assert_eq!(state.version(), 1);
    }

    #[test]
    fn contract_major_two_is_rejected_before_handler_execution() {
        assert_eq!(
            ensure_contract_compatible("2.0.0"),
            Err(SdkError::ContractIncompatible {
                required: "2.0.0".to_owned(),
                supported: SUPPORTED_CONTRACT_RANGE,
            })
        );
    }

    #[test]
    fn package_rejects_effect_value_outside_immutable_contract_enum() {
        let tenant = TenantId::new("tenant_a").unwrap();
        let schema = ArtifactRef {
            artifact_id: ArtifactId::new("schema_a").unwrap(),
            tenant_id: tenant,
            digest: sha256_digest(b"schema"),
            schema_version: "1.0.0".to_owned(),
            classification: Classification::Internal,
        };
        let bytes = b"fixture";
        let fixture = PackagedPluginFixture {
            plugin_id: PluginId::new("fixture.sdk").unwrap(),
            version: "1.0.0".to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
            publisher_id: "masonwing.first-party".to_owned(),
            execution_class: ExecutionClass::TrustedNative,
            artifact_bytes: bytes,
            artifact_digest: sha256_digest(bytes),
            handlers: vec![HandlerContract {
                id: "fixture.invoke".to_owned(),
                input_schema_ref: schema.clone(),
                output_schema_ref: schema,
                effects: "NONE".to_owned(),
                required_capabilities: Vec::new(),
                max_execution_seconds: 5,
            }],
            workflows: Vec::new(),
            supporting_artifacts: Vec::new(),
        };

        assert_eq!(
            fixture.validate(),
            Err(SdkError::InvalidPackage("handler_contract"))
        );
    }
}
