//! Composition-root adapter for effect-free SDK execution. It supplies only the
//! constant-time denying effect broker, never application/database handles.

use std::{sync::Arc, time::Duration};

use masonwing_component_runner::WasmtimeComponentRunner;
use masonwing_contracts::{Digest, PluginId, TenantId};
use masonwing_kernel::runtime::{
    CommandFailure, FailureKind, PurePluginInvocation, PurePluginOutput, PurePluginRuntime,
};
use masonwing_sdk::{DenyEffects, InvocationContext, InvocationInput, PackagedPluginFixture};
use tokio::sync::Semaphore;

use crate::plugin_execution::{NativePluginRegistration, PluginExecutionError, PluginExecutor};

pub struct HostPluginRuntime {
    executor: Arc<PluginExecutor>,
    permits: Arc<Semaphore>,
    native_packages: Vec<(PluginId, Digest)>,
}

impl HostPluginRuntime {
    pub fn new(local_fixtures: bool) -> Result<Self, CommandFailure> {
        let mut executor = PluginExecutor::new(Arc::new(WasmtimeComponentRunner::default()));
        let mut native_packages = Vec::new();
        if local_fixtures {
            let tenant =
                TenantId::new("compiled.fixture.registration").expect("static fixture tenant");
            for (fixture, plugin) in local_fixture_plugins(&tenant) {
                native_packages.push((fixture.plugin_id.clone(), fixture.artifact_digest.clone()));
                executor
                    .register_native(
                        NativePluginRegistration::from_fixture(&fixture, plugin)
                            .map_err(execution_error)?,
                    )
                    .map_err(execution_error)?;
            }
        }
        Ok(Self {
            executor: Arc::new(executor),
            permits: Arc::new(Semaphore::new(1)),
            native_packages,
        })
    }

    pub fn native_packages(&self) -> Vec<(PluginId, Digest)> {
        self.native_packages.clone()
    }
}

pub fn local_fixture_plugins(
    tenant: &TenantId,
) -> Vec<(PackagedPluginFixture, Arc<dyn masonwing_sdk::DomainPlugin>)> {
    vec![
        (
            checksum_fixture::fixture_bundle(tenant),
            Arc::new(checksum_fixture::ChecksumPlugin),
        ),
        (
            document_review_fixture::fixture_bundle(tenant),
            Arc::new(document_review_fixture::DocumentReviewPlugin),
        ),
    ]
}

#[async_trait::async_trait]
impl PurePluginRuntime for HostPluginRuntime {
    async fn invoke(
        &self,
        request: PurePluginInvocation,
    ) -> Result<PurePluginOutput, CommandFailure> {
        if !matches!(
            request.handler.effects.as_str(),
            "READ_ONLY" | "DETERMINISTIC"
        ) {
            return Err(CommandFailure::precondition("EFFECT_BROKER_REQUIRED"));
        }
        if request.input.len() > 4 * 1024 * 1024 || request.artifact_bytes.len() > 32 * 1024 * 1024
        {
            return Err(CommandFailure::invalid("PLUGIN_PAYLOAD_LIMIT"));
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| CommandFailure::new("PLUGIN_RUNTIME_BUSY", FailureKind::Exhausted))?;
        let executor = self.executor.clone();
        let maximum =
            Duration::from_secs(u64::from(request.handler.max_execution_seconds.clamp(1, 5)));
        // The permit stays with the blocking job even when its caller times out
        // or disconnects: abandoned compilation cannot accumulate unbounded jobs.
        // Only a pure result crosses back; a late result has no writer or broker.
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let context = InvocationContext::new(
                request.invocation_id,
                request.tenant_id,
                request.manifest.id.clone(),
                request.handler.id.clone(),
                request.grant_id,
                request.manifest.contract_version.clone(),
                request.handler.input_schema_ref.clone(),
                request.handler.output_schema_ref.clone(),
                request.granted_capabilities,
            )
            .map_err(|_| CommandFailure::precondition("PLUGIN_CONTEXT_INVALID"))?;
            let input =
                InvocationInput::new(request.handler.input_schema_ref.clone(), request.input);
            let result = executor
                .execute(
                    &request.manifest,
                    Some(&request.artifact_bytes),
                    &context,
                    &request.handler,
                    &input,
                    Arc::new(DenyEffects),
                )
                .map_err(execution_error)?;
            if !result.proposed_effects.is_empty() {
                return Err(CommandFailure::denied("PURE_PLUGIN_EFFECT_DENIED"));
            }
            if result.output.bytes.len() > 4 * 1024 * 1024 {
                return Err(CommandFailure::precondition("PLUGIN_OUTPUT_LIMIT"));
            }
            Ok(PurePluginOutput {
                bytes: result.output.bytes,
            })
        });
        tokio::time::timeout(maximum, job)
            .await
            .map_err(|_| CommandFailure::new("PLUGIN_DEADLINE_EXCEEDED", FailureKind::Exhausted))?
            .map_err(|_| CommandFailure::unavailable("PLUGIN_EXECUTION_FAILED"))?
    }
}

fn execution_error(error: PluginExecutionError) -> CommandFailure {
    match error {
        PluginExecutionError::RemoteWorkerUnavailable => {
            CommandFailure::unavailable("REMOTE_WORKER_UNAVAILABLE")
        }
        PluginExecutionError::ForbiddenImport => CommandFailure::denied("FORBIDDEN_IMPORT"),
        PluginExecutionError::ResourceDenied | PluginExecutionError::TrustClassDenied => {
            CommandFailure::denied("PLUGIN_CAPABILITY_DENIED")
        }
        PluginExecutionError::SandboxLimit
        | PluginExecutionError::SandboxInputLimit
        | PluginExecutionError::SandboxOutputLimit => {
            CommandFailure::new("SANDBOX_LIMIT", FailureKind::Exhausted)
        }
        _ => CommandFailure::precondition("PLUGIN_EXECUTION_INVALID"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contracts::{GrantId, ResourceId, wire::PluginManifest};
    use serde_json::json;

    fn request(fixture: PackagedPluginFixture, input: Vec<u8>) -> PurePluginInvocation {
        let manifest: PluginManifest = serde_json::from_value(json!({"id":fixture.plugin_id,"version":fixture.version,"contract_version":fixture.contract_version,
            "artifact_digest":fixture.artifact_digest,"publisher_id":fixture.publisher_id,"execution_class":fixture.execution_class,
            "requested_capabilities":[],"dependencies":[],"ui":[],"migrations":[],"license_expression":"Apache-2.0",
            "sbom_digest":masonwing_sdk::sha256_digest(b"test sbom"),"signature_ref":"test.signature","handlers":fixture.handlers,
            "workflows":fixture.workflows,"data_contracts":[],"tools":[]})).unwrap();
        PurePluginInvocation {
            invocation_id: ResourceId::new("invocation").unwrap(),
            tenant_id: manifest.handlers[0].input_schema_ref.tenant_id.clone(),
            handler: manifest.handlers[0].clone(),
            manifest,
            grant_id: GrantId::new("grant").unwrap(),
            granted_capabilities: Vec::new(),
            artifact_bytes: fixture.artifact_bytes.to_vec(),
            input,
        }
    }

    #[tokio::test]
    async fn both_native_domains_execute_canonical_payloads_through_neutral_runtime_port() {
        let runtime = HostPluginRuntime::new(true).unwrap();
        let tenant = TenantId::new("tenant_a").unwrap();
        let output = runtime
            .invoke(request(
                checksum_fixture::fixture_bundle(&tenant),
                b"[97,98,99]".to_vec(),
            ))
            .await
            .unwrap();
        let digest: String = serde_json::from_slice(&output.bytes).unwrap();
        assert_eq!(
            digest,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let output = runtime
            .invoke(request(
                document_review_fixture::fixture_bundle(&tenant),
                b"[97,98,99]".to_vec(),
            ))
            .await
            .unwrap();
        let review: String = serde_json::from_slice(&output.bytes).unwrap();
        assert_eq!(review, document_review_fixture::review_bytes(b"abc"));
    }

    #[tokio::test]
    async fn unregistered_native_and_effectful_manifests_cannot_execute() {
        let tenant = TenantId::new("tenant_a").unwrap();
        let runtime = HostPluginRuntime::new(false).unwrap();
        let error = runtime
            .invoke(request(
                checksum_fixture::fixture_bundle(&tenant),
                b"[]".to_vec(),
            ))
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "PLUGIN_CAPABILITY_DENIED");
        let runtime = HostPluginRuntime::new(true).unwrap();
        let mut input = request(checksum_fixture::fixture_bundle(&tenant), b"[]".to_vec());
        input.handler.effects = "PROPOSES_EFFECT".into();
        assert_eq!(
            runtime.invoke(input).await.err().unwrap().code,
            "EFFECT_BROKER_REQUIRED"
        );
    }

    #[tokio::test]
    async fn capacity_is_denied_before_unbounded_blocking_jobs_are_queued() {
        let runtime = HostPluginRuntime::new(true).unwrap();
        let permit = runtime.permits.clone().acquire_owned().await.unwrap();
        let tenant = TenantId::new("tenant").unwrap();
        let error = runtime
            .invoke(request(
                checksum_fixture::fixture_bundle(&tenant),
                b"[]".to_vec(),
            ))
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "PLUGIN_RUNTIME_BUSY");
        drop(permit);
        runtime
            .invoke(request(
                checksum_fixture::fixture_bundle(&tenant),
                b"[]".to_vec(),
            ))
            .await
            .unwrap();
    }
}
