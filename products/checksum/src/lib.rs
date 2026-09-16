use masonwing_sdk::{Digest, DomainPlugin, ImplementationStatus, PluginDescriptor, sha256_digest};

pub struct ChecksumPlugin;

impl DomainPlugin for ChecksumPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture.checksum",
            feature_namespaces: &["MASONWING@1.0.1:F-001"],
            operations: &["checksum.compute"],
            trace_ids: &[
                "MASONWING@1.0.1:REQ-003",
                "MASONWING@1.0.1:AC-003",
                "MASONWING@1.0.1:TC-AC-003",
            ],
            implementation_status: ImplementationStatus::NotImplemented,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumResult {
    pub digest: Digest,
    pub model_calls: u32,
    pub external_effects: u32,
}

/// Deterministic no-model fixture used to prove a second domain can consume the
/// SDK without adding domain code to the kernel.
pub fn compute(bytes: &[u8]) -> ChecksumResult {
    ChecksumResult {
        digest: sha256_digest(bytes),
        model_calls: 0,
        external_effects: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-003 / AC-003 / TC-AC-003.
    #[test]
    fn deterministic_checksum_uses_no_model_or_external_effect() {
        let first = compute(b"same input");
        let second = compute(b"same input");
        assert_eq!(first, second);
        assert_eq!(first.model_calls, 0);
        assert_eq!(first.external_effects, 0);
    }
}
