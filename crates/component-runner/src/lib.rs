//! Bounded WebAssembly Component Model execution for untrusted Masonwing plugins.
//!
//! This runner intentionally does not install WASI. A guest receives one typed
//! host import, `propose-effect`, and therefore has no ambient filesystem,
//! network, environment, stdio, clock, random, or secret access.

use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use masonwing_contracts::{Action, Digest, ResourceId};
use masonwing_sdk::{
    EffectProposal, EffectProposalHandle, HostEffectBroker, InvocationContext, InvocationInput,
    InvocationResult, SdkError,
};
use thiserror::Error;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};

pub const PLUGIN_WIT_WORLD: &str = r#"
package masonwing:plugin@1.0.0;

world plugin {
    import propose-effect: func(action: string, target: string, content-digest: string) -> result<string, string>;
    export invoke: func(handler: string, input: list<u8>) -> result<list<u8>, string>;
}
"#;

pub const L_WASM_MEMORY_BYTES: usize = 128 * 1024 * 1024;
pub const L_WASM_GUEST_DEADLINE: Duration = Duration::from_secs(5);
/// Maximum host-I/O budget required from concrete capability brokers.
///
/// The broker API is synchronous: Wasmtime fuel/epoch interruption cannot
/// preempt Rust code blocked inside a host callback. Concrete brokers therefore
/// must enforce this timeout plus cancellation/late-write safety themselves;
/// the guest watchdog alone is not evidence for AC-022.
pub const L_WASM_HOST_IO_DEADLINE: Duration = Duration::from_secs(30);
pub const DEFAULT_FUEL: u64 = 10_000_000;
pub const DEFAULT_IO_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnerLimits {
    pub memory_bytes: usize,
    pub guest_deadline: Duration,
    pub fuel: u64,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for RunnerLimits {
    fn default() -> Self {
        Self {
            memory_bytes: L_WASM_MEMORY_BYTES,
            guest_deadline: L_WASM_GUEST_DEADLINE,
            fuel: DEFAULT_FUEL,
            max_input_bytes: DEFAULT_IO_BYTES,
            max_output_bytes: DEFAULT_IO_BYTES,
        }
    }
}

impl RunnerLimits {
    pub fn validate(self) -> Result<Self, ComponentExecutionError> {
        if self.memory_bytes == 0
            || self.memory_bytes > L_WASM_MEMORY_BYTES
            || self.guest_deadline.is_zero()
            || self.guest_deadline > L_WASM_GUEST_DEADLINE
            || self.fuel == 0
            || self.max_input_bytes == 0
            || self.max_output_bytes == 0
        {
            return Err(ComponentExecutionError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Default)]
pub struct WasmtimeComponentRunner {
    limits: RunnerLimits,
}

impl WasmtimeComponentRunner {
    pub fn new(limits: RunnerLimits) -> Result<Self, ComponentExecutionError> {
        Ok(Self {
            limits: limits.validate()?,
        })
    }

    pub fn limits(&self) -> RunnerLimits {
        self.limits
    }

    pub fn invoke(
        &self,
        component_bytes: &[u8],
        context: &InvocationContext,
        handler: &masonwing_contracts::wire::HandlerContract,
        input: &InvocationInput,
        effect_broker: Arc<dyn HostEffectBroker>,
    ) -> Result<InvocationResult, ComponentExecutionError> {
        self.validate_invocation(context, handler, input)?;
        if input.bytes.len() > self.limits.max_input_bytes {
            return Err(ComponentExecutionError::InputLimit);
        }

        let engine = build_engine()?;
        let component = Component::new(&engine, component_bytes)
            .map_err(|_| ComponentExecutionError::ComponentInvalid)?;
        validate_imports(&engine, &component)?;

        let mut linker = Linker::new(&engine);
        linker
            .root()
            .func_wrap(
                "propose-effect",
                |mut store, (action, target, content_digest): (String, String, String)| {
                    let result = propose_effect(store.data(), &action, &target, &content_digest);
                    let result = match result {
                        Ok(handle) => {
                            let effect_id = handle.effect_id.to_string();
                            store.data_mut().proposed_effects.push(handle);
                            Ok(effect_id)
                        }
                        Err(code) => Err(code.to_owned()),
                    };
                    Ok((result,))
                },
            )
            .map_err(|_| ComponentExecutionError::HostLinkInvalid)?;

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(1)
            .tables(2)
            .instances(4)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(
            &engine,
            HostState {
                context: context.clone(),
                effect_broker,
                proposed_effects: Vec::new(),
                limits: store_limits,
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|_| ComponentExecutionError::RuntimeUnavailable)?;
        store.set_epoch_deadline(1);
        store.epoch_deadline_trap();

        let epoch_engine = engine.clone();
        let deadline = self
            .limits
            .guest_deadline
            .min(Duration::from_secs(u64::from(
                handler.max_execution_seconds,
            )));
        let (cancel_watchdog, watchdog_cancelled) = mpsc::channel();
        let timer = thread::spawn(move || {
            if watchdog_cancelled.recv_timeout(deadline).is_err() {
                epoch_engine.increment_epoch();
            }
        });

        let execution = (|| {
            let instance = linker
                .instantiate(&mut store, &component)
                .map_err(|error| map_runtime_error(&error))?;
            let invoke = instance
                .get_typed_func::<(String, Vec<u8>), (Result<Vec<u8>, String>,)>(
                    &mut store, "invoke",
                )
                .map_err(|_| ComponentExecutionError::ContractIncompatible)?;
            let (result,) = invoke
                .call(
                    &mut store,
                    (context.handler_id.clone(), input.bytes.clone()),
                )
                .map_err(|error| map_runtime_error(&error))?;
            result.map_err(|_| ComponentExecutionError::GuestRejected)
        })();

        let _ = cancel_watchdog.send(());
        let _ = timer.join();
        let output = execution?;
        if output.len() > self.limits.max_output_bytes {
            return Err(ComponentExecutionError::OutputLimit);
        }
        let proposed_effects = store.data().proposed_effects.clone();
        Ok(InvocationResult {
            output: masonwing_sdk::InvocationOutput::for_context(context, output),
            proposed_effects,
        })
    }

    fn validate_invocation(
        &self,
        context: &InvocationContext,
        handler: &masonwing_contracts::wire::HandlerContract,
        input: &InvocationInput,
    ) -> Result<(), ComponentExecutionError> {
        if handler.id != context.handler_id {
            return Err(ComponentExecutionError::HandlerMismatch);
        }
        if handler.max_execution_seconds == 0 {
            return Err(ComponentExecutionError::InvalidLimits);
        }
        if handler.input_schema_ref != context.input_schema_ref
            || input.schema_ref != handler.input_schema_ref
        {
            return Err(ComponentExecutionError::InputSchemaMismatch);
        }
        if handler.output_schema_ref != context.output_schema_ref {
            return Err(ComponentExecutionError::OutputSchemaMismatch);
        }
        for capability in &handler.required_capabilities {
            if !context.allows(capability) {
                return Err(ComponentExecutionError::ResourceDenied);
            }
        }
        Ok(())
    }
}

fn build_engine() -> Result<Engine, ComponentExecutionError> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|_| ComponentExecutionError::RuntimeUnavailable)
}

fn validate_imports(engine: &Engine, component: &Component) -> Result<(), ComponentExecutionError> {
    for (name, _) in component.component_type().imports(engine) {
        if name != "propose-effect" {
            return Err(ComponentExecutionError::ForbiddenImport);
        }
    }
    Ok(())
}

struct HostState {
    context: InvocationContext,
    effect_broker: Arc<dyn HostEffectBroker>,
    proposed_effects: Vec<EffectProposalHandle>,
    limits: StoreLimits,
}

fn propose_effect(
    state: &HostState,
    action: &str,
    target: &str,
    content_digest: &str,
) -> Result<EffectProposalHandle, &'static str> {
    let propose_action = Action::new("effect.propose").expect("static action is valid");
    if !state.context.allows(&propose_action) {
        return Err("CAPABILITY_DENIED");
    }
    let action = Action::new(action).map_err(|_| "INVALID_PROPOSAL")?;
    if !state.context.allows(&action) {
        return Err("RESOURCE_DENIED");
    }
    let target = ResourceId::new(target).map_err(|_| "INVALID_PROPOSAL")?;
    let content_digest = Digest::parse(content_digest).map_err(|_| "INVALID_PROPOSAL")?;
    state
        .effect_broker
        .propose_effect(EffectProposal {
            tenant_id: state.context.tenant_id.clone(),
            action,
            target,
            content_digest,
        })
        .map_err(|_| "EFFECT_PROPOSAL_DENIED")
}

fn map_runtime_error(error: &wasmtime::Error) -> ComponentExecutionError {
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel | Trap::Interrupt | Trap::MemoryOutOfBounds) => {
            ComponentExecutionError::SandboxLimit
        }
        _ => ComponentExecutionError::GuestTrap,
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ComponentExecutionError {
    #[error("COMPONENT_INVALID")]
    ComponentInvalid,
    #[error("COMPONENT_CONTRACT_INCOMPATIBLE")]
    ContractIncompatible,
    #[error("FORBIDDEN_IMPORT")]
    ForbiddenImport,
    #[error("HOST_LINK_INVALID")]
    HostLinkInvalid,
    #[error("HANDLER_MISMATCH")]
    HandlerMismatch,
    #[error("INPUT_SCHEMA_MISMATCH")]
    InputSchemaMismatch,
    #[error("OUTPUT_SCHEMA_MISMATCH")]
    OutputSchemaMismatch,
    #[error("RESOURCE_DENIED")]
    ResourceDenied,
    #[error("SANDBOX_LIMIT")]
    SandboxLimit,
    #[error("SANDBOX_INPUT_LIMIT")]
    InputLimit,
    #[error("SANDBOX_OUTPUT_LIMIT")]
    OutputLimit,
    #[error("GUEST_TRAP")]
    GuestTrap,
    #[error("GUEST_REJECTED")]
    GuestRejected,
    #[error("SANDBOX_RUNTIME_UNAVAILABLE")]
    RuntimeUnavailable,
    #[error("INVALID_SANDBOX_LIMITS")]
    InvalidLimits,
}

impl From<SdkError> for ComponentExecutionError {
    fn from(error: SdkError) -> Self {
        match error {
            SdkError::InputSchemaMismatch => Self::InputSchemaMismatch,
            SdkError::OutputSchemaMismatch => Self::OutputSchemaMismatch,
            SdkError::CapabilityDenied(_) => Self::ResourceDenied,
            SdkError::ContractIncompatible { .. } => Self::ContractIncompatible,
            _ => Self::GuestRejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use masonwing_contracts::{
        Action, ArtifactId, GrantId, ResourceId, TenantId,
        wire::{ArtifactRef, Classification, HandlerContract},
    };
    use masonwing_sdk::{
        CONTRACT_VERSION, EffectProposal, EffectProposalHandle, HostEffectBroker,
        InvocationContext, InvocationInput, SdkError, sha256_digest,
    };

    use super::*;

    #[test]
    fn real_component_round_trips_input_and_binds_output_schema() {
        let fixture = fixture(Vec::new());
        let bytes = wat::parse_str(echo_component_wat()).unwrap();
        let result = WasmtimeComponentRunner::default()
            .invoke(
                &bytes,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), b"hello".to_vec()),
                Arc::new(masonwing_sdk::DenyEffects),
            )
            .unwrap();

        assert_eq!(result.output.bytes, b"hello");
        assert_eq!(result.output.schema_ref, fixture.handler.output_schema_ref);
        assert!(result.proposed_effects.is_empty());
    }

    #[test]
    fn real_component_input_and_output_caps_fail_closed() {
        let fixture = fixture(Vec::new());
        let input_limited = WasmtimeComponentRunner::new(RunnerLimits {
            max_input_bytes: 3,
            ..RunnerLimits::default()
        })
        .unwrap();
        let echo = wat::parse_str(echo_component_wat()).unwrap();
        assert_eq!(
            input_limited.invoke(
                &echo,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), b"four".to_vec()),
                Arc::new(masonwing_sdk::DenyEffects),
            ),
            Err(ComponentExecutionError::InputLimit)
        );

        let output_limited = WasmtimeComponentRunner::new(RunnerLimits {
            max_output_bytes: 3,
            ..RunnerLimits::default()
        })
        .unwrap();
        let fixed = wat::parse_str(fixed_output_component_wat()).unwrap();
        assert_eq!(
            output_limited.invoke(
                &fixed,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), Vec::new()),
                Arc::new(masonwing_sdk::DenyEffects),
            ),
            Err(ComponentExecutionError::OutputLimit)
        );
    }

    #[test]
    fn real_component_infinite_loop_is_stopped_by_fuel() {
        let fixture = fixture(Vec::new());
        let runner = WasmtimeComponentRunner::new(RunnerLimits {
            fuel: 20_000,
            guest_deadline: Duration::from_secs(1),
            ..RunnerLimits::default()
        })
        .unwrap();
        let bytes = wat::parse_str(loop_component_wat()).unwrap();

        assert_eq!(
            runner.invoke(
                &bytes,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), Vec::new()),
                Arc::new(masonwing_sdk::DenyEffects),
            ),
            Err(ComponentExecutionError::SandboxLimit)
        );
    }

    #[test]
    fn real_component_with_ambient_import_is_rejected_before_instantiation() {
        let fixture = fixture(Vec::new());
        let bytes = wat::parse_str(
            r#"(component
                (type $run (func))
                (import "wasi:cli/run@0.2.0" (func $run-import (type $run)))
            )"#,
        )
        .unwrap();

        assert_eq!(
            WasmtimeComponentRunner::default().invoke(
                &bytes,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), Vec::new()),
                Arc::new(masonwing_sdk::DenyEffects),
            ),
            Err(ComponentExecutionError::ForbiddenImport)
        );
    }

    #[test]
    fn real_component_cannot_turn_denied_effect_proposal_into_effect_handle() {
        let effect_propose = Action::new("effect.propose").unwrap();
        let fixture_write = Action::new("fixture.write").unwrap();
        let fixture = fixture(vec![effect_propose, fixture_write]);
        let bytes = wat::parse_str(effect_component_wat()).unwrap();
        let broker = Arc::new(CountingDenyBroker::default());

        assert_eq!(
            WasmtimeComponentRunner::default().invoke(
                &bytes,
                &fixture.context,
                &fixture.handler,
                &InvocationInput::new(fixture.handler.input_schema_ref.clone(), Vec::new()),
                broker.clone(),
            ),
            Err(ComponentExecutionError::GuestRejected)
        );
        assert_eq!(broker.calls.load(Ordering::SeqCst), 1);
    }

    #[derive(Default)]
    struct CountingDenyBroker {
        calls: AtomicUsize,
    }

    impl HostEffectBroker for CountingDenyBroker {
        fn propose_effect(
            &self,
            _proposal: EffectProposal,
        ) -> Result<EffectProposalHandle, SdkError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SdkError::CapabilityDenied("effect.propose"))
        }
    }

    struct Fixture {
        context: InvocationContext,
        handler: HandlerContract,
    }

    fn fixture(capabilities: Vec<Action>) -> Fixture {
        let tenant = TenantId::new("tenant_a").unwrap();
        let input = artifact(&tenant, "schema.input", b"input-schema");
        let output = artifact(&tenant, "schema.output", b"output-schema");
        let handler = HandlerContract {
            id: "fixture.invoke".to_owned(),
            input_schema_ref: input.clone(),
            output_schema_ref: output.clone(),
            effects: "PROPOSES_EFFECT".to_owned(),
            required_capabilities: Vec::new(),
            max_execution_seconds: 5,
        };
        let context = InvocationContext::new(
            ResourceId::new("invoke_component_1").unwrap(),
            tenant,
            masonwing_contracts::PluginId::new("fixture.component").unwrap(),
            handler.id.clone(),
            GrantId::new("grant_component_1").unwrap(),
            CONTRACT_VERSION,
            input,
            output,
            capabilities,
        )
        .unwrap();
        Fixture { context, handler }
    }

    fn artifact(tenant: &TenantId, id: &str, bytes: &[u8]) -> ArtifactRef {
        ArtifactRef {
            artifact_id: ArtifactId::new(id).unwrap(),
            tenant_id: tenant.clone(),
            digest: sha256_digest(bytes),
            schema_version: "1.0.0".to_owned(),
            classification: Classification::Internal,
        }
    }

    fn echo_component_wat() -> &'static str {
        r#"(component
            (core module $guest
                (memory (export "memory") 1)
                (global $heap (mut i32) (i32.const 4096))
                (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
                    (local $ret i32)
                    global.get $heap
                    local.set $ret
                    global.get $heap
                    local.get 3
                    i32.add
                    global.set $heap
                    local.get $ret)
                (func (export "invoke") (param i32 i32 i32 i32) (result i32)
                    i32.const 1024
                    i32.const 0
                    i32.store8
                    i32.const 1028
                    local.get 2
                    i32.store
                    i32.const 1032
                    local.get 3
                    i32.store
                    i32.const 1024))
            (core instance $guest-instance (instantiate $guest))
            (alias core export $guest-instance "memory" (core memory $memory))
            (alias core export $guest-instance "cabi_realloc" (core func $realloc))
            (alias core export $guest-instance "invoke" (core func $invoke-core))
            (type $bytes (list u8))
            (type $invoke-result (result $bytes (error string)))
            (type $invoke-type (func (param "handler" string) (param "input" $bytes) (result $invoke-result)))
            (func $invoke (type $invoke-type)
                (canon lift (core func $invoke-core) (memory $memory) (realloc $realloc) string-encoding=utf8))
            (export "invoke" (func $invoke)))"#
    }

    fn fixed_output_component_wat() -> &'static str {
        r#"(component
            (core module $guest
                (memory (export "memory") 1)
                (data (i32.const 64) "WXYZ")
                (global $heap (mut i32) (i32.const 4096))
                (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
                    (local $ret i32)
                    global.get $heap
                    local.set $ret
                    global.get $heap
                    local.get 3
                    i32.add
                    global.set $heap
                    local.get $ret)
                (func (export "invoke") (param i32 i32 i32 i32) (result i32)
                    i32.const 1024
                    i32.const 0
                    i32.store8
                    i32.const 1028
                    i32.const 64
                    i32.store
                    i32.const 1032
                    i32.const 4
                    i32.store
                    i32.const 1024))
            (core instance $guest-instance (instantiate $guest))
            (alias core export $guest-instance "memory" (core memory $memory))
            (alias core export $guest-instance "cabi_realloc" (core func $realloc))
            (alias core export $guest-instance "invoke" (core func $invoke-core))
            (type $bytes (list u8))
            (type $invoke-result (result $bytes (error string)))
            (type $invoke-type (func (param "handler" string) (param "input" $bytes) (result $invoke-result)))
            (func $invoke (type $invoke-type)
                (canon lift (core func $invoke-core) (memory $memory) (realloc $realloc) string-encoding=utf8))
            (export "invoke" (func $invoke)))"#
    }

    fn loop_component_wat() -> &'static str {
        r#"(component
            (core module $guest
                (memory (export "memory") 1)
                (global $heap (mut i32) (i32.const 4096))
                (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
                    (local $ret i32)
                    global.get $heap
                    local.set $ret
                    global.get $heap
                    local.get 3
                    i32.add
                    global.set $heap
                    local.get $ret)
                (func (export "invoke") (param i32 i32 i32 i32) (result i32)
                    (loop $spin (br $spin))
                    i32.const 0))
            (core instance $guest-instance (instantiate $guest))
            (alias core export $guest-instance "memory" (core memory $memory))
            (alias core export $guest-instance "cabi_realloc" (core func $realloc))
            (alias core export $guest-instance "invoke" (core func $invoke-core))
            (type $bytes (list u8))
            (type $invoke-result (result $bytes (error string)))
            (type $invoke-type (func (param "handler" string) (param "input" $bytes) (result $invoke-result)))
            (func $invoke (type $invoke-type)
                (canon lift (core func $invoke-core) (memory $memory) (realloc $realloc) string-encoding=utf8))
            (export "invoke" (func $invoke)))"#
    }

    fn effect_component_wat() -> &'static str {
        r#"(component
            (type $effect-result (result string (error string)))
            (type $effect-type (func
                (param "action" string)
                (param "target" string)
                (param "content-digest" string)
                (result $effect-result)))
            (import "propose-effect" (func $propose-effect (type $effect-type)))
            (core module $memory-module
                (memory (export "memory") 1)
                (data (i32.const 64) "fixture.write")
                (data (i32.const 96) "target_a")
                (data (i32.const 128) "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                (global $heap (mut i32) (i32.const 4096))
                (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
                    (local $ret i32)
                    global.get $heap
                    local.set $ret
                    global.get $heap
                    local.get 3
                    i32.add
                    global.set $heap
                    local.get $ret))
            (core module $guest
                (import "env" "memory" (memory 1))
                (import "env" "propose-effect" (func $propose (param i32 i32 i32 i32 i32 i32 i32)))
                (func (export "invoke") (param i32 i32 i32 i32) (result i32)
                    i32.const 64
                    i32.const 13
                    i32.const 96
                    i32.const 8
                    i32.const 128
                    i32.const 71
                    i32.const 512
                    call $propose
                    i32.const 1024
                    i32.const 512
                    i32.load8_u
                    i32.store8
                    i32.const 1028
                    i32.const 516
                    i32.load
                    i32.store
                    i32.const 1032
                    i32.const 520
                    i32.load
                    i32.store
                    i32.const 1024))
            (core instance $memory-instance (instantiate $memory-module))
            (alias core export $memory-instance "memory" (core memory $memory))
            (alias core export $memory-instance "cabi_realloc" (core func $realloc))
            (core func $propose-core
                (canon lower (func $propose-effect) (memory $memory) (realloc $realloc) string-encoding=utf8))
            (core instance $env
                (export "memory" (memory $memory))
                (export "propose-effect" (func $propose-core)))
            (core instance $guest-instance (instantiate $guest (with "env" (instance $env))))
            (alias core export $guest-instance "invoke" (core func $invoke-core))
            (type $bytes (list u8))
            (type $invoke-result (result $bytes (error string)))
            (type $invoke-type (func (param "handler" string) (param "input" $bytes) (result $invoke-result)))
            (func $invoke (type $invoke-type)
                (canon lift (core func $invoke-core) (memory $memory) (realloc $realloc) string-encoding=utf8))
            (export "invoke" (func $invoke)))"#
    }
}
