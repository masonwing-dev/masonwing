use masonwing_contracts::{AdapterQualification, ArtifactId, Digest};
use masonwing_kernel::ports::{AdapterBoundary, ArtifactPort, PortError};

#[derive(Debug, Default)]
pub struct ArtifactStoreAdapter;

impl AdapterBoundary for ArtifactStoreAdapter {
    fn adapter_name(&self) -> &'static str {
        "artifacts"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl ArtifactPort for ArtifactStoreAdapter {
    fn metadata_digest(&self, _artifact_id: &ArtifactId) -> Result<Digest, PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
