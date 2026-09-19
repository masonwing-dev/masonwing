# Durable execution: runs, approvals, effects and budget

Implementation update: 17 September 2026. The immutable requirements, generated
contracts and applied migrations retain their original identities and thresholds.
This report describes application behavior and its verification scope, not full
product acceptance or release approval.

## Scope and status

Work package WP-011..014 is implemented on top of the existing PostgreSQL
schema (migrations 0003 and 0004) and is exercised end to end locally against
real PostgreSQL and real MinIO. **Product acceptance remains `NOT_ACCEPTED`**:
553 of the 686 baseline acceptance cases remain RED; 133 pass under
`tests/acceptance/drivers.py` (1 TC-AC [TC-AC-073], 131 TC-API [43 missing_auth,
43 cross_tenant including TC-API-042, 43 invalid_input, 2 idempotency_mismatch],
1 TC-ST [ST-0118: run QUEUED->CANCELLED via cancel_before_start]).
TC-AC-072 is driven over the live HTTP wire but stays RED on its honest "một root execution"
oracle (requiring durable-engine acknowledgement, `dispatch_state=STARTED` + non-empty
`temporal_run_id`). 41 idempotency_mismatch cases stay RED:
12 operations have no store handler (prime cannot reach a receipt) and 29 lack
real canonical inputs in the driver — these stay RED on the implementation gap,
not on a relaxed oracle.
313 state_model cases remain RED: 312 plugin vectors blocked by step-up
infrastructure (all plugin lifecycle commands require a qualifying LoA-2
`acr`/`amr` that the local Keycloak realm does not issue); 1 run vector
(ST-0118) passes. 24 quality cases and 14 pairwise cases remain RED (their
evidence requires independent review and real Cedar/domain execution
respectively), and the per-case product acceptance suite is tracked
separately from this implementation evidence.

The `membership.accept` cross_tenant deviation was fixed in
`crates/host-api/src/runtime.rs` + `crates/host-api/src/auth/session.rs`:
before the bounded body read, the route tenant must exist and be ACTIVE
(`SessionAuthority::tenant_admission`), so foreign/absent tenants now get the
same masked 404 as every other command (TC-API-042 GREEN, and all 43
cross_tenant cases now pass). The signed-invite binding itself is still proved
inside `execute_membership_accept_bootstrap`; tenant admission only restores the
baseline absent-resource shape ahead of it.

The durable engine adapter (`platform-plugins/workflow-temporal`) and the
external-mutation broker (`platform-plugins/connections`) remain
`AdapterQualification::Unqualified`. No live Temporal dispatch and no external
provider transmission is claimed.

## Command surface

Every command below reaches a PostgreSQL handler through the same boundary used
by the registry slice: schema validation, canonical fingerprint, tenant advisory
lock with RLS variables set, membership/permission/policy epoch re-check,
authority resolution from host rows, idempotent replay against
`command_receipts`, and one transaction that persists the resource projection,
audit records, outbox events and the receipt together.

| Command | Handler | Durable shape |
|---|---|---|
| `run.start` | `run_commands.rs::start_run` | `runs` QUEUED, `dispatch_state=PENDING`, ACCEPTED + non-null `run_id` |
| `run.cancel` | `run_commands.rs::cancel_run` | terminal `CANCELLED`, or `CANCEL_REQUESTED` after transmit; fence increments |
| `approval.decide` | `approval_commands.rs::decide_approval` | one winner from `REQUESTED`; four-eyes refusal for the author |
| `effect.propose` | `effect_commands.rs::propose_effect` | `effects` PREPARED + `cost_reservations` RESERVED, committed before transport |
| `effect.dispatch` | `effect_commands.rs::dispatch_effect` | `EXECUTING` with `transmit_count`/`dispatch_fence` increment |
| `effect.reconcile` | `effect_commands.rs::reconcile_effect` | `RECONCILING` with `reconciliation_queued` |
| `effect.compensate` | `effect_commands.rs::compensate_effect` | a **new** effect linked by `original_effect_id` |
| `budget.configure` | `administration.rs::configure_budget` | per-period ceiling, rejected below outstanding commitments |

## Run identity and admission (REQ-069..073)

`run.start` resolves the exact `workflow_definitions` row for the requested
`(workflow_id, workflow_version)`. There is no default workflow: a missing or
corrupt definition is a request error. The definition pins a plugin digest, and
only that digest's installation may admit new runs — `DRAINING`, `REVOKED` and
`DISABLED` installations are refused before the row is written.

The run input artifact is verified against the workflow's pinned
`input_schema_ref` **before** the `runs` row exists, so an invalid input never
creates durable work. The `runs.temporal_workflow_id` column is `UNIQUE` and is
set to the run's own UUID, which makes the durable engine's identity stable
across retries: a second start attempt for the same logical run cannot create a
second engine execution.

Idempotency is scoped to `(tenant, principal, operation, idempotency_key)` in
`command_receipts`. Replaying the identical command returns the original receipt
unchanged; the same key with a different fingerprint is a 409 conflict.

Two baseline acceptance cases are driven over the real HTTP wire against the
live Compose stack (`tests/acceptance/drivers.py`), through a real Keycloak
session and a fully seeded tenant (plugin installation, signed manifest,
conforming input artifact, active grant). **One passes, one stays RED on the
honest oracle.**

| Case | Status | Assertion |
|---|---|---|
| `TC-AC-072` (AC-072) | **RED** | Two `POST run.start` with the same `(K, P)` both return 202 with a byte-identical replay and the **same** `run_id`; exactly one `runs` row, one receipt, one audit record. The oracle **requires one root execution**, which means durable-engine acknowledgement (`dispatch_state=STARTED`, non-empty `temporal_run_id`). This compose build has no qualified Temporal adapter (`start_durable_run` is compiled out without `local-temporal-tests` and fails closed), so `dispatch_state` stays `PENDING` and the case **stays RED** rather than passing on a PostgreSQL row count. |
| `TC-AC-073` (AC-073) | **GREEN** | Same `K` with a different fingerprint (changed `workflow_version`): 409 `IDEMPOTENCY_CONFLICT`, `effect_state` `NOT_SENT`, `retryable` false; run count still 1 with the original `run_id`; still one receipt and one audit record |

`run.cancel` follows the state machine rather than a blanket stop. A run that
has not yet transmitted moves straight to `CANCELLED`; a run that has transmitted
moves to `CANCEL_REQUESTED` and the fence increments, so the in-flight worker's
next checkpoint write is refused as `STALE_FENCE` instead of resurrecting the
cancelled traversal.

## Crash-resume and checkpoints (REQ-070, REQ-074, REQ-075, REQ-077)

`run_checkpoints` is keyed `(tenant_id, run_id, logical_step_id)`. The logical
step id is the ordered dispatcher key (`0001:<node>`, `0002:<node>`), which makes
"the same step" a durable name rather than a position in an in-memory plan.

Three invariants are enforced inside the checkpoint transaction:

- A checkpoint whose `state` is already `SUCCEEDED` is **never rewritten** to a
  pending state. A re-issued completion of the same logical step returns
  `false` and changes nothing, so a restarted worker cannot re-run completed
  work.
- A checkpoint carrying a fence that no longer matches the run is refused with
  `STALE_FENCE`, so a worker that lost its lease cannot drive an outdated
  traversal.
- A replay that arrives with the same logical step id but a **different node
  identity** is an incompatible replay (REQ-075): the step is quarantined as
  `BLOCKED` with `REPLAY_INCOMPATIBLE` and nothing is transmitted.

The pure graph interpreter lives in `crates/worker/src/run_interpreter.rs` and
is deliberately separate from the database. `crates/worker/src/run_resume.rs`
bridges `DurableCheckpoint` rows to it, so resumed execution is derived from
committed history instead of worker memory.

`runs.max_total_steps`/`max_model_turns` are enforced through `BoundedLoop`;
`LIMIT_REACHED` halts dispatch without a further tool or model call. A node
output is validated against its pinned `output_schema_ref` before it may be
forwarded, so an invalid output blocks downstream execution rather than
propagating.

## Approval admission (REQ-078..083)

`approval.decide` loads the proposal `FOR UPDATE`, and the update itself is
conditional on `state='REQUESTED'`. Under the row lock only one decider can win;
every loser observes a terminal state and receives `DECISION_CONFLICT` rather
than overwriting the winner's decision.

- `decided_by == created_by` is refused with `FOUR_EYES_REQUIRED` (403).
- A decision whose `content_digest` differs from the stored binding digest is
  refused with `REVIEW_CONFLICT`: the reviewer must decide over exactly the
  content that was previewed.
- A `REQUESTED` proposal past `expires_at` is closed, not decided:
  `APPROVAL_EXPIRED`.
- `INVALIDATED` (a changed binding) returns `APPROVAL_STALE`.

A decision transmits nothing. It terminates the proposal and lets the durable
workflow observe the terminal state through the outbox event recorded with the
projection change.

## Effects: intent first, evidence only (REQ-084..091)

`effect.propose` commits the durable `effects` row and its budget reservation
**before** any transport can happen. This is what makes "never blind resend"
enforceable rather than aspirational: the intent exists, so a retry is
reconciliation against committed state instead of a second transmission.

- `effect.dispatch` re-checks the kill switch, refuses a resend from
  `OUTCOME_UNKNOWN`/`RECONCILING`/`MANUAL_REVIEW`, requires an `APPROVED` and
  unexpired approval when one is bound, and only then moves PREPARED→EXECUTING
  while incrementing `transmit_count` and `dispatch_fence`.
- The budget hold is deliberately **not** released at dispatch. It is settled
  only when remote evidence arrives, so funds cannot be released before the
  mutation is known to have been applied.
- An ambiguous outcome becomes `OUTCOME_UNKNOWN` with `reconciliation_queued`
  set and the reservation held as unknown usage.
- `record_effect_succeeded` writes the `effect_receipts` row with its evidence
  reference and settles the reservation in the same transaction.
- `effect.compensate` creates a **new** effect linked through
  `original_effect_id` and requires its own separate approval; a compensation is
  never an edit of the original row.

The same `(tenant_id, connection_id, idempotency_key)` triple with a different
payload fingerprint is refused with 409 `IDEMPOTENCY_CONFLICT`; the same triple
with the same fingerprint returns the original projection.

## Budget ledger (REQ-092..098)

`reserve_budget` increments `budget_accounts.held_microunits` and relies on the
table's `CHECK (held_microunits <= limit_microunits - charged_microunits)`
constraint as the arbiter. There is no select-then-check window: concurrent
reservations that would exceed the ceiling violate the constraint and are mapped
to `BUDGET_EXCEEDED`.

An unresolvable price profile never reaches the ledger: a non-positive bounded
price is refused with `PRICE_BOUND_UNKNOWN`. Settlement is once-only — the
reservation's settled flag and the `cost_ledger` uniqueness constraint make a
second settlement idempotent. Unknown usage keeps `held` in place until
reconciliation, and release is reserved for proven absence.

## Verification

Locked evidence: `.evidence/runtime-20260917-125642-048791/verification.json`
(`source_unchanged_during_checks: true`), covering Rust formatting, workspace
tests, the PostgreSQL registry and crash-resume suites, the authenticated HTTP
command suite, and clippy under `-D warnings`. Acceptance-driver evidence:
`.venv/bin/python -m pytest tests/acceptance/test_requirements.py -q`
→ 133 passed (TC-AC-073, 131 TC-API including all 43 cross_tenant
[TC-API-042 fixed], TC-ST-0118), 553 failed (TC-AC-072 stays
honestly RED on durable-engine acknowledgement; 41 idempotency_mismatch stay
RED on the
implementation gap; 313 state_model stay RED: 312 plugin on step-up
infrastructure, 1 run on the engine-acknowledgement gap; 24 quality and 14
pairwise stay RED), 0 skipped, 0 errors (2026-09-18).

| Proof | Command | Asserts |
|---|---|---|
| Workspace unit/contract tests | `cargo test --locked --workspace --all-targets --jobs 2` | kernel guards, interpreter, resume bridge |
| Registry handlers on PostgreSQL | `cargo test --locked -p masonwing-data-postgres --features local-postgres-tests --test registry_postgres --jobs 2` | registry slice handlers |
| Crash-resume on PostgreSQL (AC-074) | `cargo test --locked -p masonwing-worker --features local-postgres-tests --test crash_resume_postgres --jobs 2` | interrupted step re-issued under its logical id; completed step skipped; stale worker refused |
| Durable-execution end to end | `cargo run --locked -p masonwing-host-api --example local_run_flow -- --local` | see below |
| Registry/native end to end | `cargo run --locked -p masonwing-host-api --example local_plugin_flow -- --local` | registry + invocation slice |
| Authenticated HTTP command surface | `.venv/bin/python -m pytest tests/integration/test_run_commands.py -q` | see below |
| Full local check set | `python3 scripts/verify_runtime.py contracts format rust postgres runs native runflow runhttp lint` | all of the above + drift + clippy |

`local_run_flow` asserts, on committed durable state in real PostgreSQL/MinIO:
`run.start` returns ACCEPTED with a non-null `run_id`; replaying the identical
command returns the identical receipt; `mark_run_started` takes the run to
RUNNING; a `SUCCEEDED` checkpoint is not rewritten by a duplicate write; the
checkpoint history contains the completed step; `effect.propose` reserves budget
and commits the intent before dispatch; the author's self-decision is refused
(four-eyes) and an independent approver's decision succeeds; `effect.dispatch`
returns ACCEPTED into EXECUTING; a recorded provider receipt drives the effect
to `SUCCEEDED` and settles 15 microunits; `mark_run_completed` finishes the run
at `SUCCEEDED`. The report records `model_calls: 0` and
`external_provider_mutations: 0`.

| HTTP admission surface | `.venv/bin/python -m pytest tests/integration/test_run_commands.py -q` | 42 cases over a real Keycloak Authorization Code + PKCE session: 401 for all 8 durable commands unauthenticated; 403 CSRF (missing token, foreign origin); 404 before body parsing on foreign tenants; 400 `SCHEMA_INVALID` on empty bodies; 403 `STEP_UP_REQUIRED` on `budget.configure`; 404 on non-existent targets |
| Rust boundary receipts | `cargo run --locked -p masonwing-host-api --example local_run_flow -- --local` | `run.start` ACCEPTED + non-null `run_id` + idempotent replay equality, `approval.decide` FOUR_EYES, `effect.propose/dispatch` ACCEPTED and SUCCEEDED settlement |

The HTTP integration covers admission and the error shapes the durable commands
produce at the wire level; the business-level 202/`run_id` receipt semantics are
asserted through the Rust store boundary (`local_run_flow`), because a fully
provisioned workflow grant plus plugin installation requires operator-level
seeded state (digests, signatures, active grant membership) that the ephemeral
tenant fixture does not exercise over HTTP. This is not a contract gap: the
receipt shape is pinned by the generated `CommandReceipt` schema, and the HTTP
route always returns 200/202 with that shape after store execution.

## Deliberately not claimed

- **TC-AC-072 "một root execution" stays RED without durable-engine proof.**
  The driver's same-receipt/same-run-ID assertions pass, but its engine-ack
  oracle does not: `dispatch_state` stays `PENDING`, `temporal_run_id` empty.
  One PostgreSQL row is insufficient evidence of one root execution; that
  earlier weakened interpretation has been removed. The case will only pass
  when a real Temporal execution is acknowledged by the stable run identity.
- **Live Temporal dispatch.** The worker binary runs the outbox relay and the
  compose health surface, but `start_durable_run` is compiled out unless the
  opt-in `local-temporal-tests` feature is enabled, and otherwise fails closed
  with `DURABLE_ENGINE_UNAVAILABLE`. No run in this slice is driven by Temporal,
  so "started a workflow" is not asserted anywhere.
- **External provider transmission.** Effects reach `EXECUTING` and are then
  given a recorded receipt. `ConnectionBrokerAdapter::transmit_after_current_guards`
  still returns `NotQualified`; no HTTP mutation is performed.
- **Approval proposal creation.** The approval row in the example is seeded as a
  Given, because the product path that raises a proposal is the workflow's
  waiting-approval step. Only the decision itself runs through the real command.
- **Per-tenant fairness scheduling** (REQ-097) and **object GC**.
- **Product acceptance.** `tests/acceptance` remains RED by design.