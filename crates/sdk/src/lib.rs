//! Public SDK surface for Masonwing domain plugins.
//!
//! The SDK deliberately exposes effect *proposal* capability, never a direct
//! provider mutation primitive. The host remains the authority and effect broker.

use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub use masonwing_contracts::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub id: &'static str,
    pub feature_namespaces: &'static [&'static str],
    pub operations: &'static [&'static str],
    pub trace_ids: &'static [&'static str],
    pub implementation_status: ImplementationStatus,
}

pub trait DomainPlugin: Send + Sync {
    fn descriptor(&self) -> PluginDescriptor;

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
pub trait HostEffectBroker {
    fn propose_effect(&self, proposal: EffectProposal) -> Result<EffectProposalHandle, SdkError>;
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
}
