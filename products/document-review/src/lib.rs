use masonwing_sdk::{
    ArtifactId, ArtifactRef, CONTRACT_VERSION, Classification, Digest, DomainPlugin,
    ExecutionClass, HandlerContract, HostEffectBroker, InvocationContext, InvocationInput,
    InvocationResult, PackagedArtifact, PackagedPluginFixture, PluginDescriptor, PluginId,
    SdkError, TenantId, sha256_digest,
    wire::{WorkflowDefinition, WorkflowNode},
};

pub const PLUGIN_ID: &str = "fixture.document-review";
pub const HANDLER_ID: &str = "document-review.review";
pub const VERSION: &str = "1.0.0";
pub const PACKAGED_BYTES: &[u8] = b"masonwing:first-party-native-fixture\0fixture.document-review\0v1.0.0\0document-review.review\0";

const INPUT_SCHEMA_JSON: &[u8] = br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","items":{"maximum":255,"minimum":0,"type":"integer"},"title":"Document review input bytes","type":"array"}"#;
const OUTPUT_SCHEMA_JSON: &[u8] = br#"{"$schema":"https://json-schema.org/draft/2020-12/schema","pattern":"^review:v1;bytes=[0-9]+;digest=sha256:[0-9a-f]{64};verdict=REVIEWED$","title":"Deterministic document review summary","type":"string"}"#;

pub struct DocumentReviewPlugin;

impl DomainPlugin for DocumentReviewPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: PLUGIN_ID,
            contract_version: CONTRACT_VERSION,
            feature_namespaces: &["MASONWING@1.0.1:F-001"],
            operations: &[HANDLER_ID],
            trace_ids: &[
                "MASONWING@1.0.1:REQ-002",
                "MASONWING@1.0.1:AC-002",
                "MASONWING@1.0.1:TC-AC-002",
            ],
        }
    }

    fn invoke(
        &self,
        context: &InvocationContext,
        input: &InvocationInput,
        _host: &dyn HostEffectBroker,
    ) -> Result<InvocationResult, SdkError> {
        self.validate_invocation(context, input)?;
        let payload: Vec<u8> = serde_json::from_slice(&input.bytes)
            .map_err(|_| SdkError::InvalidPayload("input_json"))?;
        let review = review_bytes(&payload);
        let output = serde_json_canonicalizer::to_vec(&review)
            .expect("serializing a review string cannot fail");
        Ok(InvocationResult::pure(context, output))
    }
}

pub fn review_bytes(bytes: &[u8]) -> String {
    let digest: Digest = sha256_digest(bytes);
    format!(
        "review:v1;bytes={};digest={};verdict=REVIEWED",
        bytes.len(),
        digest.as_str()
    )
}

pub fn fixture_bundle(tenant_id: &TenantId) -> PackagedPluginFixture {
    let supporting_artifacts = fixture_artifacts(tenant_id);
    let input = supporting_artifacts[0].reference.clone();
    let output = supporting_artifacts[1].reference.clone();
    let workflow = supporting_artifacts[2].reference.clone();
    PackagedPluginFixture {
        plugin_id: PluginId::new(PLUGIN_ID).expect("static plugin ID is valid"),
        version: VERSION.to_owned(),
        contract_version: CONTRACT_VERSION.to_owned(),
        publisher_id: "masonwing.first-party".to_owned(),
        execution_class: ExecutionClass::TrustedNative,
        artifact_bytes: PACKAGED_BYTES,
        artifact_digest: sha256_digest(PACKAGED_BYTES),
        handlers: vec![HandlerContract {
            id: HANDLER_ID.to_owned(),
            input_schema_ref: input,
            output_schema_ref: output,
            effects: "DETERMINISTIC".to_owned(),
            required_capabilities: Vec::new(),
            max_execution_seconds: 5,
        }],
        workflows: vec![workflow],
        supporting_artifacts,
    }
}

/// Deterministic canonical schema/workflow payloads for host operator seeding.
pub fn fixture_artifacts(tenant_id: &TenantId) -> Vec<PackagedArtifact> {
    let input_bytes = canonical_json(INPUT_SCHEMA_JSON);
    let output_bytes = canonical_json(OUTPUT_SCHEMA_JSON);
    let input = artifact_ref(
        tenant_id,
        "fixture.document-review.schema.input.v1",
        &input_bytes,
    );
    let output = artifact_ref(
        tenant_id,
        "fixture.document-review.schema.output.v1",
        &output_bytes,
    );
    let workflow_definition = WorkflowDefinition {
        id: "fixture.document-review.workflow.v1".to_owned(),
        version: "1.0.0".to_owned(),
        input_schema_ref: input.clone(),
        output_schema_ref: output.clone(),
        entry_node: "document-review.review".to_owned(),
        nodes: vec![WorkflowNode {
            id: "document-review.review".to_owned(),
            kind: "DETERMINISTIC".to_owned(),
            handler_id: Some(HANDLER_ID.to_owned()),
            input_schema_ref: input.clone(),
            output_schema_ref: output.clone(),
            max_attempts: 1,
            timeout_seconds: 5,
            max_iterations: 1,
            on_failure: "FAIL".to_owned(),
        }],
        edges: Vec::new(),
        max_total_steps: 1,
        max_model_turns: 0,
        required_actions: Vec::new(),
        digest: sha256_digest(b"workflow-digest-placeholder"),
    };
    let workflow_bytes = canonical_workflow(workflow_definition);
    let workflow = artifact_ref(
        tenant_id,
        "fixture.document-review.workflow.v1",
        &workflow_bytes,
    );

    vec![
        PackagedArtifact {
            reference: input,
            bytes: input_bytes,
        },
        PackagedArtifact {
            reference: output,
            bytes: output_bytes,
        },
        PackagedArtifact {
            reference: workflow,
            bytes: workflow_bytes,
        },
    ]
}

fn canonical_json(raw: &[u8]) -> Vec<u8> {
    let value: serde_json::Value =
        serde_json::from_slice(raw).expect("static JSON schema is valid");
    serde_json_canonicalizer::to_vec(&value).expect("static JSON schema canonicalizes")
}

fn canonical_workflow(mut definition: WorkflowDefinition) -> Vec<u8> {
    let mut value = serde_json::to_value(&definition).expect("static fixture workflow serializes");
    value
        .as_object_mut()
        .expect("WorkflowDefinition serializes as an object")
        .remove("digest");
    let identity_bytes = serde_json_canonicalizer::to_vec(&value)
        .expect("workflow identity canonicalizes without digest");
    definition.digest = sha256_digest(&identity_bytes);
    serde_json_canonicalizer::to_vec(&definition).expect("static fixture workflow canonicalizes")
}

fn artifact_ref(tenant_id: &TenantId, id: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        artifact_id: ArtifactId::new(id).expect("static fixture artifact ID is valid"),
        tenant_id: tenant_id.clone(),
        digest: sha256_digest(bytes),
        schema_version: "1.0.0".to_owned(),
        classification: Classification::Internal,
    }
}

#[cfg(test)]
mod tests {
    use masonwing_sdk::{DenyEffects, GrantId, ResourceId};

    use super::*;

    #[test]
    fn review_is_deterministic_and_kernel_independent() {
        assert_eq!(review_bytes(b"policy text"), review_bytes(b"policy text"));
        assert!(review_bytes(b"policy text").ends_with("verdict=REVIEWED"));
    }

    #[test]
    fn packaged_fixture_has_stable_bytes_handlers_workflow_and_sdk_invocation() {
        let tenant = TenantId::new("tenant_a").unwrap();
        let bundle = fixture_bundle(&tenant);
        bundle.validate().unwrap();
        assert_eq!(bundle, fixture_bundle(&tenant));
        assert_eq!(bundle.artifact_digest, sha256_digest(PACKAGED_BYTES));
        assert_eq!(bundle.supporting_artifacts, fixture_artifacts(&tenant));
        for artifact in &bundle.supporting_artifacts {
            artifact.validate().unwrap();
        }
        let input_schema: serde_json::Value =
            serde_json::from_slice(&bundle.supporting_artifacts[0].bytes).unwrap();
        assert_eq!(
            input_schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        let workflow: WorkflowDefinition =
            serde_json::from_slice(&bundle.supporting_artifacts[2].bytes).unwrap();
        assert_eq!(workflow.nodes[0].handler_id.as_deref(), Some(HANDLER_ID));
        assert_eq!(workflow.max_model_turns, 0);
        assert_eq!(
            workflow.digest,
            workflow_identity_digest(&bundle.supporting_artifacts[2].bytes)
        );

        let handler = &bundle.handlers[0];
        let context = InvocationContext::new(
            ResourceId::new("invoke_review_1").unwrap(),
            tenant,
            bundle.plugin_id.clone(),
            HANDLER_ID,
            GrantId::new("grant_review_1").unwrap(),
            CONTRACT_VERSION,
            handler.input_schema_ref.clone(),
            handler.output_schema_ref.clone(),
            Vec::new(),
        )
        .unwrap();
        let payload = serde_json_canonicalizer::to_vec(&b"policy text".to_vec()).unwrap();
        let result = DocumentReviewPlugin
            .invoke(
                &context,
                &InvocationInput::new(handler.input_schema_ref.clone(), payload),
                &DenyEffects,
            )
            .unwrap();
        let review: String = serde_json::from_slice(&result.output.bytes).unwrap();
        assert_eq!(review, review_bytes(b"policy text"));
        assert!(result.proposed_effects.is_empty());
    }

    fn workflow_identity_digest(bytes: &[u8]) -> Digest {
        let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        value.as_object_mut().unwrap().remove("digest");
        sha256_digest(&serde_json_canonicalizer::to_vec(&value).unwrap())
    }
}
