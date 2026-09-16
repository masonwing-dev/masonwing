# Masonwing Rust scaffold

This document describes the executable Rust scaffold derived from the immutable
`masonwing-requirements-v1.0.1` and `gleanbird-requirements-v1.0.1` baselines.
It is implementation evidence for the scaffold only. It does not mark any
baseline work package, product acceptance case, live provider, or production
policy as complete.

## Workspace shape

The workspace is pinned to Rust `1.97.1`, edition 2024, and contains 28 packages.
`Cargo.lock` is generated from the real crates.io registry rather than copied
from the requirement inputs.

The required base crates are:

- `crates/contracts`: domain-neutral public value objects and the closed
  `MASONWING@1.0.1` operation catalog.
- `crates/kernel`: host-side invariants, finite-state transitions, typed
  application ports, registry resolution, delegation, approvals, effects,
  budget retention, and bounded loops.
- `crates/sdk`: the public domain-plugin API. Domain plugins can propose effects
  through a host capability but cannot transmit provider mutations directly.
- `crates/host-api`: Axum service scaffold and fail-closed command gate.
- `crates/worker`, `crates/component-runner`, `crates/remote-runner`: runnable
  service processes sharing the same health/status and graceful-shutdown
  runtime while execution remains `NOT_IMPLEMENTED`.
- `crates/cli`: the `masonwing` local CLI with `status` and `version` only.

The current registry lock resolves the key direct dependencies to Axum 0.8.9,
Tokio 1.53.1, Serde 1.0.229, thiserror 2.0.20, SHA-2 0.10.9, and Tower 0.5.3.
The lock file is the source of truth for the complete transitive set.

## Runtime contract

The service binaries are:

| Package | Binary / process name |
| --- | --- |
| `masonwing-host-api` | `masonwing-host-api` |
| `masonwing-worker` | `masonwing-worker` |
| `masonwing-component-runner` | `masonwing-component-runner` |
| `masonwing-remote-runner` | `masonwing-remote-runner` |
| `masonwing-cli` | `masonwing` |

HTTP processes use `MASONWING_BIND_ADDR`, defaulting to `0.0.0.0:8080`.
They handle `SIGTERM` and Ctrl-C through Axum graceful shutdown and do not run a
poll/spin loop.

`GET /health/live` returns 200 when the process is serving. `GET /health/ready`
returns 503 in this scaffold because critical write adapters are unqualified.
Liveness therefore remains useful for a local container while readiness does
not falsely assert that protected writes are safe.

`GET /dev/status` is exposed only for `MASONWING_ENV=LOCAL` and has this shape:

```json
{
  "service": "masonwing-worker",
  "environment": "LOCAL",
  "scaffold": true,
  "implementation_status": "NOT_IMPLEMENTED",
  "external_mutation_enabled": false,
  "live_budget_microunits": 0,
  "readiness": {
    "writes": false,
    "critical_adapters_qualified": false
  },
  "contract_version": "1.0.0"
}
```

`STAGING_LIVE` and `PRODUCTION` startup is rejected while this scaffold remains
unqualified. This prevents a local/mock runtime from becoming a live deployment
through environment configuration alone.

The OpenAPI command family is represented by
`POST /v1/tenants/{tenant_id}/commands/{operation}`. The scaffold checks for a
credential reference using headers before it extracts or parses request body
data. Missing authentication returns 401 with `effect_state=NOT_SENT`. A bearer
or session-cookie-shaped value is still not treated as verified identity: until
the OIDC adapter is qualified, the result is 503
`IDENTITY_ADAPTER_UNAVAILABLE`. No command path returns a synthetic successful
`CommandReceipt`.

## Implemented kernel invariants

These are executable scaffold invariants rather than product acceptance claims.

| Runtime behavior | Baseline trace | Executable evidence |
| --- | --- | --- |
| Tenant-scoped resource rejects conflicting authenticated tenant | `MASONWING@1.0.1:REQ-047`, `AC-049` | `scope::tests::authenticated_tenant_wins_over_conflicting_resource_scope` |
| Plugin capability cannot widen a run grant | `MASONWING@1.0.1:REQ-050`, `AC-052`, `TC-AC-052` | `grants::tests::wider_plugin_capability_does_not_expand_run_grant` |
| Grant is expired at `now == expires_at` | `MASONWING@1.0.1:REQ-052`, `AC-054`, `TC-AC-054` | `grants::tests::grant_is_expired_at_exact_expiry_instant` |
| Child grant cannot exceed parent resources/actions/expiry | `MASONWING@1.0.1:REQ-054`, `AC-056`, `TC-AC-056` | `grants::tests::child_grant_cannot_expand_parent_resource_set` |
| Approval captures exact action/target/digest/revision/scope/actor | `MASONWING@1.0.1:REQ-078`, `AC-082`, `TC-AC-082` | `approval::tests::approval_record_keeps_exact_immutable_binding` |
| Changed content after approval is stale | `MASONWING@1.0.1:REQ-079`, `AC-083`, `TC-AC-083` | `approval::tests::changed_content_digest_invalidates_approval` |
| Approval expires at exact boundary | `MASONWING@1.0.1:REQ-080`, `AC-084`, `TC-AC-084` | `approval::tests::approval_expires_at_exact_boundary` |
| Ambiguous provider timeout becomes `OUTCOME_UNKNOWN`, queues reconciliation, and blocks resend | `MASONWING@1.0.1:REQ-085`, `AC-089`, `TC-AC-089` | `effects::tests::ambiguous_timeout_queues_reconciliation_and_never_blind_resends` |
| Unresolved reconciliation becomes manual review without resend | `MASONWING@1.0.1:REQ-087`, `AC-091`, `TC-AC-091` | `effects::tests::unresolved_reconciliation_moves_to_manual_review_without_resend` |
| Unknown provider usage retains the reservation instead of coercing it to zero | `MASONWING@1.0.1:REQ-094`, `AC-098`, `TC-AC-098` | `budget::tests::unknown_usage_retains_full_reservation_until_reconciled` |
| Agent loop stops before dispatch after the configured turn bound | `MASONWING@1.0.1:REQ-072`, `AC-076` | `loop_guard::tests::dispatch_callback_is_never_called_after_turn_limit` |
| Dependency cycle is detected before migrations or installed-registry changes | `MASONWING@1.0.1:REQ-007`, `AC-007`, `TC-AC-007` | `registry::tests::dependency_cycle_is_rejected_before_migration_or_registry_change` |
| Terminal run cannot resume and version remains unchanged | `MASONWING@1.0.1:REQ-073`, `AC-077`, `TC-AC-077` | `state::tests::terminal_run_cannot_resume_and_version_does_not_change` |

The integration test `crates/kernel/tests/state_vectors.rs` loads the immutable
`MASONWING@1.0.1` state-vector file and executes all 314 declared ordered pairs
against the runtime transition oracle. Legal edges change state and increment
the version; illegal edges return `ILLEGAL_TRANSITION` and preserve both. That
test validates finite-state adjacency only. The separate tests above exercise
guards such as expiry, immutable binding, no resend, and unknown-cost retention.

## Platform feature boundaries

The Rust layer exposes structural boundaries for all Masonwing feature slices
without pretending their infrastructure exists.

| Feature slice | Rust boundary / current status |
| --- | --- |
| `F-001` composition | `CompositionRecipe` / `CompositionPort`; product descriptors through SDK; infrastructure not implemented |
| `F-002` plugin identity/dependency | `DependencyGraph` and cycle-safe install planning implemented at kernel level |
| `F-003` lifecycle/upgrade/revoke | plugin finite-state oracle implemented; real package verification/migration not implemented |
| `F-004` isolated execution | `ComponentExecutionPort` plus runnable component/remote runner processes; Wasmtime/remote execution not implemented |
| `F-005` identity | `IdentityPort`, `platform-plugins/identity-oidc`; adapter unqualified |
| `F-006` session/membership | `MembershipPort`; OIDC/BFF session implementation not implemented |
| `F-007` authorization/tenancy | tenant scope guard, `AuthorizationPort`, Cedar adapter; Cedar adapter unqualified |
| `F-008` machine delegation | `RunGrant` subset/expiry guards implemented |
| `F-009` data/atomic audit | `DataPort`, `AuditPort`; PostgreSQL adapter unqualified |
| `F-010` artifacts | `ArtifactPort`; artifact adapter unqualified |
| `F-011` durable workflow | `DurableWorkflowPort`, bounded-loop guard; Temporal adapter unqualified |
| `F-012` immutable approval | exact approval-binding guards implemented |
| `F-013` effects/reconciliation | effect state and no-resend guards implemented; `ExternalMutationPort` unqualified |
| `F-014` budget/quota | reservation/unknown retention implemented; `BudgetPort` adapter unqualified |
| `F-015` model providers | `ModelPort`; provider adapter unqualified |
| `F-016` connectors/secrets | `ConnectionPort`, `SecretPort`; both adapters unqualified |
| `F-017` scheduler/events | `SchedulerPort`, `EventPort`; no custom scheduler implementation; event adapter unqualified |
| `F-018` UI contributions | `UiContributionPort`; browser implementation lives outside this Rust slice |
| `F-019` query/notification/export | `ProjectionQueryPort`, `NotificationPort`, `ExportPort`; implementations not present |
| `F-020` audit/support | `AuditPort`, `SupportAccessPort`; implementations not present |
| `F-021` release/operations | `ReleaseQualificationPort`; live qualification not implemented |
| `F-022` catalog/marketplace | `CatalogPort`; marketplace implementation not present |
| `F-023` SDK/conformance | public `masonwing-sdk` plus `ConformancePort`; product conformance suite not implemented |

The infrastructure crates under `platform-plugins/` compile against these typed
kernel ports but report `AdapterQualification::Unqualified` and `PortError::NotQualified`:

`identity-oidc`, `authorization-cedar`, `data-postgres`, `artifacts`,
`workflow-temporal`, `model-provider`, `connections`, `budget`, `events`, and
`secrets`.

They do not contain homemade OIDC, authorization, scheduler, database, secret,
or provider implementations. Real implementations must wrap and qualify the
approved dependencies from the baseline before readiness can become true.

## Gleanbird SDK-only domain packages

The Gleanbird Rust packages use `masonwing-sdk` as their only runtime workspace
dependency and do not import `masonwing-kernel` or any `platform-plugins` crate.
The measurement package additionally uses Serde/Serde JSON as test-only
dependencies to read the immutable vector fixture. The eight packages expose the
complete operation catalog while command handlers remain `NOT_IMPLEMENTED`:

| Package | Gleanbird feature grouping |
| --- | --- |
| `gleanbird-brand-truth` | `F-001`–`F-002` |
| `gleanbird-sources-voc` | `F-003`–`F-005` |
| `gleanbird-opportunities` | `F-006`–`F-007` |
| `gleanbird-evidence-content` | `F-008`–`F-012` |
| `gleanbird-delivery` | `F-013`–`F-014` |
| `gleanbird-measurement-optimization` | `F-015`–`F-017`, `F-020`–`F-022` |
| `gleanbird-media` | `F-018` M3 extension |
| `gleanbird-distribution` | `F-019` M3 extension |

No domain package receives raw secrets or a direct provider-transmit API. The
SDK's external-mutation capability is `HostEffectBroker::propose_effect`; the
host retains dispatch authority.

### Pure-domain starter behavior

Several deterministic value-object rules are implemented to make the scaffold
TDD-ready before infrastructure work. These tests assert pure domain outputs and
do not claim the end-to-end acceptance criteria are complete.

| Domain behavior | Baseline trace | Implemented output |
| --- | --- | --- |
| SCORE-1 missing factor | `GLEANBIRD@1.0.1:REQ-034`, `AC-034`, `TC-AC-034` | any missing factor makes the full score `null`; missing names and present-weight subtotal remain visible |
| SCORE-1 known vector | `GLEANBIRD@1.0.1:reference-vectors.json` | factors `80,60,40,20,100,80` with risk penalty 5 produce exactly `59.0` |
| Source-rights hard block | `GLEANBIRD@1.0.1:REQ-039`, `AC-039`, `TC-AC-039` | revoked/unlicensed rights return `SOURCE_RIGHTS_DENIED` semantics regardless of a high score |
| Protected manual block | `GLEANBIRD@1.0.1:REQ-056`, `AC-056`, `TC-AC-056` | AI suggestion does not modify a protected block; attempted delta is retained separately for explicit review |
| Stale editor revision | `GLEANBIRD@1.0.1:REQ-057`, `AC-057`, `TC-AC-057` | stale base version returns both base/current revision references; current acknowledged revision remains unchanged |
| Mention denominator | `GLEANBIRD@1.0.1:REQ-096`, `AC-096`, `TC-AC-096` | 12 mentions / 24 eligible = 0.5, failed=6, completion=24/30=0.8 |
| Zero eligible sample | `GLEANBIRD@1.0.1:REQ-097`, `AC-097`, `TC-AC-097` | value is `null`, state is `NO_DATA`, denominator is 0; no NaN or synthetic zero-performance value |
| Wilson95 | `GLEANBIRD@1.0.1:REQ-099`, `AC-099`, `TC-AC-099` | 12/24 uses z=1.959963984540054 and yields `[0.3142742581957335, 0.6857257418042665]`, carrying `n=24` and method name |

The public SDK also contains a generic `FiniteStateContract`; Gleanbird supplies
its own domain edge tables in a product test without importing kernel internals.
`products/gleanbird/measurement-optimization/tests/state_vectors.rs` executes all
294 immutable `GLEANBIRD@1.0.1` state vectors. Legal pairs update state/version;
illegal pairs return `ILLEGAL_TRANSITION` and preserve both. As with the Masonwing
vector test, this proves declared adjacency behavior only. Rights, scoring,
revision, denominator, and other guards require their separate domain tests.

## Second-domain fixtures

`products/document-review` is an SDK-only domain skeleton whose review command
returns explicit `NOT_IMPLEMENTED`. It provides the second-domain composition
boundary required by `MASONWING@1.0.1:REQ-002` / `AC-002` without adding document
semantics to the kernel.

`products/checksum` is a deterministic no-model fixture. `compute` returns a
contract-format SHA-256 digest with `model_calls=0` and `external_effects=0`.
The executable test is traced to `MASONWING@1.0.1:REQ-003`, `AC-003`, and
`TC-AC-003`.

## Verification and limits

The bounded Rust checks for this scaffold are:

```text
cargo check --workspace -j 2
cargo test --workspace -j 2
cargo fmt --all -- --check
```

The current Rust scaffold has passed 41 Rust tests. One integration test executes all 314 immutable
Masonwing state vectors; another executes all 294 immutable Gleanbird state
vectors through the public SDK state contract. These vectors are exercised inside
two tests, not counted as 608 additional product acceptance tests. Current command
results and logs are recorded in `.dev/evidence/verification.json`; the full
cross-stack evidence index is `docs/verification.md`.

The earlier local process smoke built all five executable entry points. Each
HTTP process was started on an isolated loopback port in the reserved
39850–39859 development range, observed with `live=200`, `ready=503`, and the
truthful LOCAL scaffold status above, then terminated with `SIGTERM`. All four
processes exited cleanly and released their listener. The `masonwing status` CLI
returned the same `NOT_IMPLEMENTED`, mutation-disabled, zero-live-budget status.

Earlier entry-point smoke and current Rust test scope:

```text
cargo fmt --all -- --check                         PASS
cargo check --workspace -j 2                       PASS
cargo test --locked --workspace -j 2               PASS (41 tests, 0 failed)
cargo build -p masonwing-host-api \
  -p masonwing-worker \
  -p masonwing-component-runner \
  -p masonwing-remote-runner -j 2                  PASS
cargo build -p masonwing-cli -j 2                  PASS

masonwing-host-api       live=200 ready=503 status=truthful sigterm=clean
masonwing-worker         live=200 ready=503 status=truthful sigterm=clean
masonwing-component-runner live=200 ready=503 status=truthful sigterm=clean
masonwing-remote-runner  live=200 ready=503 status=truthful sigterm=clean
masonwing CLI            status=truthful
```

The following Rust adapters remain deliberately unimplemented: Keycloak/OIDC verification,
Cedar policy evaluation, SQLx repositories and incremental migration runner, artifact backend,
Temporal workflows/schedules, Wasmtime component execution, remote-worker
protocol, model providers, OAuth/MCP connectors, OpenBao secret resolution,
provider effect transmission, live budgets, dynamic plugin UI loading, search/export/support,
release qualification, marketplace/catalog behavior, Gleanbird product
workflow persistence/audit/provider wiring, and live/production evidence.
The Compose services, initial PostgreSQL RLS/transaction/audit DDL, private versioned
S3 storage, scoped OpenBao fixtures and React route shell are present and have
their own actual integration/browser tests. Those tests qualify these local
scaffold mechanisms, not the missing application adapters or full product flows.
The pure Gleanbird functions above prove only their deterministic output rules;
they do not establish the full AC evidence bundles. Scaffold presence must not
be reported as product PASS or acceptance completion.
