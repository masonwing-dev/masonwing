use masonwing_sdk::{
    ArtifactId, DomainPlugin, ImplementationStatus, PluginDescriptor, SdkError, TenantId,
};

pub struct DocumentReviewPlugin;

impl DomainPlugin for DocumentReviewPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture.document-review",
            feature_namespaces: &["MASONWING@1.0.1:F-001"],
            operations: &["document-review.review"],
            trace_ids: &["MASONWING@1.0.1:REQ-002", "MASONWING@1.0.1:AC-002"],
            implementation_status: ImplementationStatus::NotImplemented,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRequest {
    pub tenant_id: TenantId,
    pub artifact_id: ArtifactId,
}

impl DocumentReviewPlugin {
    pub fn review(&self, _request: ReviewRequest) -> Result<(), SdkError> {
        self.invoke_scaffold("document-review.review")
    }
}
