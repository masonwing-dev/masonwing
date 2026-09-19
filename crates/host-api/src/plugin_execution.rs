//! Host-owned plugin execution selection.
//!
//! The registry/lifecycle boundary must authenticate and verify the manifest
//! before it reaches this module. Execution still rechecks immutable identity,
//! handler binding and artifact digest so a routing bug cannot silently change
//! the code that is executed.

use std::{collections::BTreeMap, sync::Arc};

use masonwing_component_runner::{ComponentExecutionError, WasmtimeComponentRunner};
use masonwing_contracts::wire::{ExecutionClass, HandlerContract, PluginManifest};
use masonwing_sdk::{
    DomainPlugin, HostEffectBroker, InvocationContext, InvocationInput, InvocationResult,
    PackagedPluginFixture, SdkError, ensure_contract_compatible, sha256_digest,
};
use thiserror::Error;

/// Wasm execution port implemented by the isolated component-runner crate.
///
/// The host owns the dispatch port and depends on the isolated component-runner
/// implementation. The runner itself deliberately has no host-api dependency,
/// which keeps the dependency direction acyclic.
pub trait WasmExecutor: Send + Sync {
    fn invoke(
        &self,
        component_bytes: &[u8],
        context: &InvocationContext,
        handler: &HandlerContract,
        input: &InvocationInput,
        effect_broker: Arc<dyn HostEffectBroker>,
    ) -> Result<InvocationResult, PluginExecutionError>;
}

impl WasmExecutor for WasmtimeComponentRunner {
    fn invoke(
        &self,
        component_bytes: &[u8],
        context: &InvocationContext,
        handler: &HandlerContract,
        input: &InvocationInput,
        effect_broker: Arc<dyn HostEffectBroker>,
    ) -> Result<InvocationResult, PluginExecutionError> {
        WasmtimeComponentRunner::invoke(
            self,
            component_bytes,
            context,
            handler,
            input,
            effect_broker,
        )
        .map_err(PluginExecutionError::from)
    }
}

/// Registration token for a compiled, first-party native plugin.
///
/// There is intentionally no constructor from arbitrary strings/digests. A
/// native registration can only be derived from the deterministic fixture
/// package material compiled into the host build.
pub struct NativePluginRegistration {
    plugin_id: String,
    artifact_digest: String,
    plugin_version: String,
    contract_version: String,
    publisher_id: String,
    handlers: BTreeMap<String, HandlerContract>,
    plugin: Arc<dyn DomainPlugin>,
}

impl NativePluginRegistration {
    pub fn from_fixture(
        fixture: &PackagedPluginFixture,
        plugin: Arc<dyn DomainPlugin>,
    ) -> Result<Self, PluginExecutionError> {
        fixture.validate()?;
        if fixture.execution_class != ExecutionClass::TrustedNative {
            return Err(PluginExecutionError::TrustClassDenied);
        }

        let descriptor = plugin.descriptor();
        if descriptor.id != fixture.plugin_id.as_str() {
            return Err(PluginExecutionError::PluginIdentityMismatch);
        }
        if descriptor.contract_version != fixture.contract_version {
            return Err(PluginExecutionError::ContractBindingMismatch);
        }
        if descriptor.operations.len() != fixture.handlers.len()
            || fixture
                .handlers
                .iter()
                .any(|handler| !descriptor.operations.contains(&handler.id.as_str()))
        {
            return Err(PluginExecutionError::HandlerContractMismatch);
        }

        Ok(Self {
            plugin_id: fixture.plugin_id.to_string(),
            artifact_digest: fixture.artifact_digest.to_string(),
            plugin_version: fixture.version.clone(),
            contract_version: fixture.contract_version.clone(),
            publisher_id: fixture.publisher_id.clone(),
            handlers: fixture
                .handlers
                .iter()
                .map(|handler| (handler.id.clone(), handler.clone()))
                .collect(),
            plugin,
        })
    }

    fn key(&self) -> (String, String) {
        (self.plugin_id.clone(), self.artifact_digest.clone())
    }

    fn validates_manifest(&self, manifest: &PluginManifest) -> bool {
        let submitted: std::collections::BTreeSet<_> = manifest
            .handlers
            .iter()
            .map(|handler| &handler.id)
            .collect();
        manifest.version == self.plugin_version
            && manifest.contract_version == self.contract_version
            && manifest.publisher_id == self.publisher_id
            && submitted.len() == manifest.handlers.len()
            && self.handlers.len() == submitted.len()
            && manifest.handlers.iter().all(|handler| {
                self.handlers.get(&handler.id).is_some_and(|compiled| {
                    // Operator import remaps tenant-local handles. Content identity
                    // and validation/effect semantics must still match compiled code.
                    same_schema_content(&compiled.input_schema_ref, &handler.input_schema_ref)
                        && same_schema_content(
                            &compiled.output_schema_ref,
                            &handler.output_schema_ref,
                        )
                        && compiled.effects == handler.effects
                        && compiled.required_capabilities == handler.required_capabilities
                        && compiled.max_execution_seconds == handler.max_execution_seconds
                })
            })
    }
}

fn same_schema_content(a: &masonwing_sdk::ArtifactRef, b: &masonwing_sdk::ArtifactRef) -> bool {
    a.digest == b.digest
        && a.schema_version == b.schema_version
        && a.classification == b.classification
}

/// Neutral host dispatch table. It does not know product/domain names.
pub struct PluginExecutor {
    native: BTreeMap<(String, String), NativePluginRegistration>,
    wasm: Arc<dyn WasmExecutor>,
}

impl PluginExecutor {
    pub fn new(wasm: Arc<dyn WasmExecutor>) -> Self {
        Self {
            native: BTreeMap::new(),
            wasm,
        }
    }

    pub fn register_native(
        &mut self,
        registration: NativePluginRegistration,
    ) -> Result<(), PluginExecutionError> {
        let key = registration.key();
        if self.native.contains_key(&key) {
            return Err(PluginExecutionError::DuplicateNativeRegistration);
        }
        self.native.insert(key, registration);
        Ok(())
    }

    /// Execute a previously verified manifest using its declared trust class.
    ///
    /// `artifact_bytes` is required only for WASM_COMPONENT. TRUSTED_NATIVE is
    /// selected exclusively from the fixed registration table and never loads
    /// tenant/package bytes into the host process. REMOTE_WORKER remains
    /// unavailable until an authenticated leased transport is qualified.
    pub fn execute(
        &self,
        manifest: &PluginManifest,
        artifact_bytes: Option<&[u8]>,
        context: &InvocationContext,
        handler: &HandlerContract,
        input: &InvocationInput,
        effect_broker: Arc<dyn HostEffectBroker>,
    ) -> Result<InvocationResult, PluginExecutionError> {
        validate_binding(manifest, context, handler, input)?;

        match manifest.execution_class {
            ExecutionClass::TrustedNative => {
                let key = (
                    manifest.id.to_string(),
                    manifest.artifact_digest.to_string(),
                );
                let registration = self
                    .native
                    .get(&key)
                    .ok_or(PluginExecutionError::TrustClassDenied)?;
                if !registration.validates_manifest(manifest) {
                    return Err(PluginExecutionError::HandlerContractMismatch);
                }
                let result = registration
                    .plugin
                    .invoke(context, input, effect_broker.as_ref())?;
                result.validate_output(context)?;
                Ok(result)
            }
            ExecutionClass::WasmComponent => {
                let bytes = artifact_bytes.ok_or(PluginExecutionError::ArtifactRequired)?;
                if sha256_digest(bytes) != manifest.artifact_digest {
                    return Err(PluginExecutionError::ArtifactDigestMismatch);
                }
                let result = self
                    .wasm
                    .invoke(bytes, context, handler, input, effect_broker)?;
                result.validate_output(context)?;
                Ok(result)
            }
            ExecutionClass::RemoteWorker => Err(PluginExecutionError::RemoteWorkerUnavailable),
        }
    }
}

fn validate_binding(
    manifest: &PluginManifest,
    context: &InvocationContext,
    handler: &HandlerContract,
    input: &InvocationInput,
) -> Result<(), PluginExecutionError> {
    ensure_contract_compatible(&manifest.contract_version)?;
    if manifest.id != context.plugin_id {
        return Err(PluginExecutionError::PluginIdentityMismatch);
    }
    if manifest.contract_version != context.contract_version {
        return Err(PluginExecutionError::ContractBindingMismatch);
    }
    if handler.id != context.handler_id
        || handler.input_schema_ref.tenant_id != context.tenant_id
        || handler.output_schema_ref.tenant_id != context.tenant_id
        || handler.input_schema_ref != context.input_schema_ref
        || handler.output_schema_ref != context.output_schema_ref
        || input.schema_ref != handler.input_schema_ref
        || !manifest
            .handlers
            .iter()
            .any(|candidate| candidate == handler)
    {
        return Err(PluginExecutionError::HandlerContractMismatch);
    }
    if handler
        .required_capabilities
        .iter()
        .any(|action| !context.allows(action) || !manifest.requested_capabilities.contains(action))
    {
        return Err(PluginExecutionError::ResourceDenied);
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PluginExecutionError {
    #[error("TRUST_CLASS_DENIED")]
    TrustClassDenied,
    #[error("PLUGIN_IDENTITY_MISMATCH")]
    PluginIdentityMismatch,
    #[error("CONTRACT_BINDING_MISMATCH")]
    ContractBindingMismatch,
    #[error("HANDLER_CONTRACT_MISMATCH")]
    HandlerContractMismatch,
    #[error("PLUGIN_ARTIFACT_REQUIRED")]
    ArtifactRequired,
    #[error("PLUGIN_ARTIFACT_DIGEST_MISMATCH")]
    ArtifactDigestMismatch,
    #[error("REMOTE_WORKER_UNAVAILABLE")]
    RemoteWorkerUnavailable,
    #[error("DUPLICATE_NATIVE_REGISTRATION")]
    DuplicateNativeRegistration,
    #[error("SANDBOX_LIMIT")]
    SandboxLimit,
    #[error("SANDBOX_INPUT_LIMIT")]
    SandboxInputLimit,
    #[error("SANDBOX_OUTPUT_LIMIT")]
    SandboxOutputLimit,
    #[error("FORBIDDEN_IMPORT")]
    ForbiddenImport,
    #[error("RESOURCE_DENIED")]
    ResourceDenied,
    #[error("WASM_EXECUTION_FAILED:{0}")]
    WasmExecutionFailed(&'static str),
    #[error(transparent)]
    Sdk(#[from] SdkError),
}

impl From<ComponentExecutionError> for PluginExecutionError {
    fn from(error: ComponentExecutionError) -> Self {
        match error {
            ComponentExecutionError::SandboxLimit => Self::SandboxLimit,
            ComponentExecutionError::InputLimit => Self::SandboxInputLimit,
            ComponentExecutionError::OutputLimit => Self::SandboxOutputLimit,
            ComponentExecutionError::ForbiddenImport => Self::ForbiddenImport,
            ComponentExecutionError::ResourceDenied => Self::ResourceDenied,
            ComponentExecutionError::ContractIncompatible => {
                Self::WasmExecutionFailed("COMPONENT_CONTRACT_INCOMPATIBLE")
            }
            _ => Self::WasmExecutionFailed("SANDBOX_EXECUTION_FAILED"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use masonwing_contracts::{
        Action, ArtifactId, Digest, GrantId, PluginId, ResourceId, TenantId,
        wire::{ArtifactRef, Classification, PluginManifest},
    };
    use masonwing_sdk::{
        CONTRACT_VERSION, DenyEffects, InvocationOutput, PluginDescriptor, sha256_digest,
    };

    use super::*;

    static NATIVE_CALLS: AtomicUsize = AtomicUsize::new(0);
    static WASM_CALLS: AtomicUsize = AtomicUsize::new(0);

    struct NativeFixture;

    impl DomainPlugin for NativeFixture {
        fn descriptor(&self) -> PluginDescriptor {
            PluginDescriptor {
                id: "fixture.native",
                contract_version: CONTRACT_VERSION,
                feature_namespaces: &["MASONWING@1.0.1:F-004"],
                operations: &["fixture.invoke"],
                trace_ids: &["MASONWING@1.0.1:AC-019"],
            }
        }

        fn invoke(
            &self,
            context: &InvocationContext,
            input: &InvocationInput,
            _host: &dyn HostEffectBroker,
        ) -> Result<InvocationResult, SdkError> {
            self.validate_invocation(context, input)?;
            NATIVE_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResult::pure(context, input.bytes.clone()))
        }
    }

    #[derive(Default)]
    struct WasmFixture;

    impl WasmExecutor for WasmFixture {
        fn invoke(
            &self,
            _component_bytes: &[u8],
            context: &InvocationContext,
            _handler: &HandlerContract,
            input: &InvocationInput,
            _effect_broker: Arc<dyn HostEffectBroker>,
        ) -> Result<InvocationResult, PluginExecutionError> {
            WASM_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResult {
                output: InvocationOutput::for_context(context, input.bytes.clone()),
                proposed_effects: Vec::new(),
            })
        }
    }

    #[test]
    fn tenant_native_digest_cannot_select_compiled_plugin() {
        NATIVE_CALLS.store(0, Ordering::SeqCst);
        let fixture = native_fixture();
        let mut executor = PluginExecutor::new(Arc::new(WasmFixture));
        executor
            .register_native(
                NativePluginRegistration::from_fixture(&fixture, Arc::new(NativeFixture)).unwrap(),
            )
            .unwrap();

        let handler = fixture.handlers[0].clone();
        let mut manifest = manifest_for(&fixture);
        manifest.artifact_digest = digest_of(b"tenant supplied native bytes");
        let context = context_for(&manifest, &handler);
        let input = InvocationInput::new(handler.input_schema_ref.clone(), b"payload".to_vec());

        assert_eq!(
            executor.execute(
                &manifest,
                Some(b"tenant supplied native bytes"),
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects),
            ),
            Err(PluginExecutionError::TrustClassDenied)
        );
        assert_eq!(NATIVE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wasm_digest_is_verified_before_runner_receives_bytes() {
        WASM_CALLS.store(0, Ordering::SeqCst);
        let fixture = native_fixture();
        let handler = fixture.handlers[0].clone();
        let mut manifest = manifest_for(&fixture);
        manifest.execution_class = ExecutionClass::WasmComponent;
        manifest.artifact_digest = digest_of(b"expected wasm");
        let context = context_for(&manifest, &handler);
        let input = InvocationInput::new(handler.input_schema_ref.clone(), Vec::new());
        let executor = PluginExecutor::new(Arc::new(WasmFixture));

        assert_eq!(
            executor.execute(
                &manifest,
                Some(b"different wasm"),
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects),
            ),
            Err(PluginExecutionError::ArtifactDigestMismatch)
        );
        assert_eq!(WASM_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn remote_worker_is_explicitly_unavailable_without_transport() {
        let fixture = native_fixture();
        let handler = fixture.handlers[0].clone();
        let mut manifest = manifest_for(&fixture);
        manifest.execution_class = ExecutionClass::RemoteWorker;
        let context = context_for(&manifest, &handler);
        let input = InvocationInput::new(handler.input_schema_ref.clone(), Vec::new());
        let executor = PluginExecutor::new(Arc::new(WasmFixture));

        assert_eq!(
            executor.execute(
                &manifest,
                None,
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects),
            ),
            Err(PluginExecutionError::RemoteWorkerUnavailable)
        );
    }

    #[test]
    fn incompatible_contract_is_rejected_before_native_handler_execution() {
        NATIVE_CALLS.store(0, Ordering::SeqCst);
        let fixture = native_fixture();
        let mut executor = PluginExecutor::new(Arc::new(WasmFixture));
        executor
            .register_native(
                NativePluginRegistration::from_fixture(&fixture, Arc::new(NativeFixture)).unwrap(),
            )
            .unwrap();
        let handler = fixture.handlers[0].clone();
        let mut manifest = manifest_for(&fixture);
        let context = context_for(&manifest, &handler);
        manifest.contract_version = "2.0.0".to_owned();
        let input = InvocationInput::new(handler.input_schema_ref.clone(), Vec::new());

        assert_eq!(
            executor.execute(
                &manifest,
                None,
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects),
            ),
            Err(PluginExecutionError::Sdk(SdkError::ContractIncompatible {
                required: "2.0.0".to_owned(),
                supported: masonwing_sdk::SUPPORTED_CONTRACT_RANGE,
            }))
        );
        assert_eq!(NATIVE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn schema_binding_mismatch_is_rejected_before_native_handler_execution() {
        NATIVE_CALLS.store(0, Ordering::SeqCst);
        let fixture = native_fixture();
        let mut executor = PluginExecutor::new(Arc::new(WasmFixture));
        executor
            .register_native(
                NativePluginRegistration::from_fixture(&fixture, Arc::new(NativeFixture)).unwrap(),
            )
            .unwrap();
        let handler = fixture.handlers[0].clone();
        let manifest = manifest_for(&fixture);
        let context = context_for(&manifest, &handler);
        let wrong_schema = artifact(
            &handler.input_schema_ref.tenant_id,
            "schema.other",
            b"different-schema",
        );
        let input = InvocationInput::new(wrong_schema, Vec::new());

        assert_eq!(
            executor.execute(
                &manifest,
                None,
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects),
            ),
            Err(PluginExecutionError::HandlerContractMismatch)
        );
        assert_eq!(NATIVE_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn native_compiled_contract_rejects_rebinding_but_accepts_remapped_handles() {
        struct CountedNative(Arc<AtomicUsize>);
        impl DomainPlugin for CountedNative {
            fn descriptor(&self) -> PluginDescriptor {
                NativeFixture.descriptor()
            }
            fn invoke(
                &self,
                context: &InvocationContext,
                input: &InvocationInput,
                _: &dyn HostEffectBroker,
            ) -> Result<InvocationResult, SdkError> {
                self.validate_invocation(context, input)?;
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(InvocationResult::pure(context, input.bytes.clone()))
            }
        }
        let fixture = native_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut executor = PluginExecutor::new(Arc::new(WasmFixture));
        executor
            .register_native(
                NativePluginRegistration::from_fixture(
                    &fixture,
                    Arc::new(CountedNative(calls.clone())),
                )
                .unwrap(),
            )
            .unwrap();
        let original = manifest_for(&fixture);
        let mut variants = Vec::new();
        let mut value = original.clone();
        value.handlers[0].output_schema_ref.digest = digest_of(b"changed schema");
        variants.push(value);
        let mut value = original.clone();
        value.handlers[0].effects = "PROPOSES_EFFECT".into();
        variants.push(value);
        let mut value = original.clone();
        value.handlers[0].max_execution_seconds = 10;
        variants.push(value);
        let mut value = original.clone();
        value.handlers.push(value.handlers[0].clone());
        variants.push(value);
        let mut value = original.clone();
        value.version = "9.0.0".into();
        variants.push(value);
        for value in variants {
            let handler = value.handlers[0].clone();
            let context = context_for(&value, &handler);
            let input = InvocationInput::new(handler.input_schema_ref.clone(), b"[]".to_vec());
            assert_eq!(
                executor.execute(
                    &value,
                    None,
                    &context,
                    &handler,
                    &input,
                    Arc::new(DenyEffects)
                ),
                Err(PluginExecutionError::HandlerContractMismatch)
            );
        }
        let mut changed_caps = original.clone();
        let action = Action::new("effect.propose").unwrap();
        changed_caps.requested_capabilities.push(action.clone());
        changed_caps.handlers[0]
            .required_capabilities
            .push(action.clone());
        let handler = changed_caps.handlers[0].clone();
        let context = InvocationContext::new(
            ResourceId::new("invocation").unwrap(),
            handler.input_schema_ref.tenant_id.clone(),
            changed_caps.id.clone(),
            handler.id.clone(),
            GrantId::new("grant").unwrap(),
            CONTRACT_VERSION,
            handler.input_schema_ref.clone(),
            handler.output_schema_ref.clone(),
            [action],
        )
        .unwrap();
        let input = InvocationInput::new(handler.input_schema_ref.clone(), b"[]".to_vec());
        assert_eq!(
            executor.execute(
                &changed_caps,
                None,
                &context,
                &handler,
                &input,
                Arc::new(DenyEffects)
            ),
            Err(PluginExecutionError::HandlerContractMismatch)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let mut remapped = original;
        for handler in &mut remapped.handlers {
            handler.input_schema_ref.artifact_id =
                ArtifactId::new("11111111-1111-4111-8111-111111111111").unwrap();
            handler.output_schema_ref.artifact_id =
                ArtifactId::new("22222222-2222-4222-8222-222222222222").unwrap();
            handler.input_schema_ref.tenant_id = TenantId::new("another-tenant").unwrap();
            handler.output_schema_ref.tenant_id = TenantId::new("another-tenant").unwrap();
        }
        let handler = &remapped.handlers[0];
        let context = context_for(&remapped, handler);
        let input = InvocationInput::new(handler.input_schema_ref.clone(), b"[]".to_vec());
        executor
            .execute(
                &remapped,
                None,
                &context,
                handler,
                &input,
                Arc::new(DenyEffects),
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn required_capability_is_checked_before_native_or_wasm_dispatch() {
        let fixture = native_fixture();
        let mut executor = PluginExecutor::new(Arc::new(WasmFixture));
        executor
            .register_native(
                NativePluginRegistration::from_fixture(&fixture, Arc::new(NativeFixture)).unwrap(),
            )
            .unwrap();
        for execution in [ExecutionClass::TrustedNative, ExecutionClass::WasmComponent] {
            let mut manifest = manifest_for(&fixture);
            manifest.execution_class = execution;
            let action = Action::new("artifact.read").unwrap();
            manifest.requested_capabilities.push(action.clone());
            manifest.handlers[0].required_capabilities.push(action);
            let handler = &manifest.handlers[0];
            let context = context_for(&manifest, handler);
            let input = InvocationInput::new(handler.input_schema_ref.clone(), Vec::new());
            assert_eq!(
                executor.execute(
                    &manifest,
                    Some(fixture.artifact_bytes),
                    &context,
                    handler,
                    &input,
                    Arc::new(DenyEffects)
                ),
                Err(PluginExecutionError::ResourceDenied)
            );
        }
    }

    fn native_fixture() -> PackagedPluginFixture {
        let tenant = TenantId::new("tenant_a").unwrap();
        let input = artifact(&tenant, "schema.input", b"input-schema");
        let output = artifact(&tenant, "schema.output", b"output-schema");
        let bytes = b"fixed native package";
        PackagedPluginFixture {
            plugin_id: PluginId::new("fixture.native").unwrap(),
            version: "1.0.0".to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
            publisher_id: "masonwing.first-party".to_owned(),
            execution_class: ExecutionClass::TrustedNative,
            artifact_bytes: bytes,
            artifact_digest: sha256_digest(bytes),
            handlers: vec![HandlerContract {
                id: "fixture.invoke".to_owned(),
                input_schema_ref: input,
                output_schema_ref: output,
                effects: "READ_ONLY".to_owned(),
                required_capabilities: Vec::new(),
                max_execution_seconds: 5,
            }],
            workflows: Vec::new(),
            supporting_artifacts: Vec::new(),
        }
    }

    fn manifest_for(fixture: &PackagedPluginFixture) -> PluginManifest {
        PluginManifest {
            id: fixture.plugin_id.clone(),
            version: fixture.version.clone(),
            contract_version: fixture.contract_version.clone(),
            artifact_digest: fixture.artifact_digest.clone(),
            publisher_id: fixture.publisher_id.clone(),
            execution_class: fixture.execution_class,
            requested_capabilities: Vec::<Action>::new(),
            dependencies: Vec::new(),
            ui: Vec::new(),
            migrations: Vec::new(),
            license_expression: "Apache-2.0".to_owned(),
            sbom_digest: digest_of(b"sbom"),
            signature_ref: "fixture.signature".to_owned(),
            handlers: fixture.handlers.clone(),
            workflows: fixture.workflows.clone(),
            data_contracts: Vec::new(),
            tools: Vec::new(),
        }
    }

    fn context_for(manifest: &PluginManifest, handler: &HandlerContract) -> InvocationContext {
        InvocationContext::new(
            ResourceId::new("invoke_fixture_1").unwrap(),
            handler.input_schema_ref.tenant_id.clone(),
            manifest.id.clone(),
            handler.id.clone(),
            GrantId::new("grant_fixture_1").unwrap(),
            manifest.contract_version.clone(),
            handler.input_schema_ref.clone(),
            handler.output_schema_ref.clone(),
            Vec::<Action>::new(),
        )
        .unwrap()
    }

    fn artifact(tenant: &TenantId, id: &str, bytes: &[u8]) -> ArtifactRef {
        ArtifactRef {
            artifact_id: ArtifactId::new(id).unwrap(),
            tenant_id: tenant.clone(),
            digest: digest_of(bytes),
            schema_version: "1.0.0".to_owned(),
            classification: Classification::Internal,
        }
    }

    fn digest_of(bytes: &[u8]) -> Digest {
        sha256_digest(bytes)
    }
}
