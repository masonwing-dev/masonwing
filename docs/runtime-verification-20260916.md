# Runtime implementation verification — 16 September 2026

## Outcome and scope

The current local application executes OIDC browser login, tenant/CSRF admission,
scanned confidential artifact upload/finalize/readback, signed plugin registry and
lifecycle commands, exact product composition and synchronous pure plugin
invocation. Both checksum and document-review compiled fixtures have produced
verified output using the same host runtime and actual PostgreSQL/MinIO adapters.

This is not completion of the entire immutable SRS. The local baseline contains
23 features, 158 requirements, 162 acceptance criteria and 686 designed cases.
The acceptance-driver registry is still empty. The checks below retain their
unit/application/infrastructure scope and do not mark those acceptance cases PASS.
Readiness intentionally remains HTTP 503 `QUALIFICATION_INCOMPLETE`; external
provider mutations remain disabled and live budget is zero.

## Observed checks

| Check | Observed result | Evidence |
| --- | --- | --- |
| Final Rust workspace/all targets | 105 passed, 0 failed; one separately gated MinIO test ignored by default | `.evidence/runtime-20260916-133151-410464/rust-workspace.log` |
| Final Rust format and Clippy | Both passed; Clippy uses `-D warnings` | `.evidence/runtime-20260916-133151-410464/verification.json` |
| Registry/invocation/upload-provenance PostgreSQL integration | 3 passed, real PostgreSQL; instrumented object/runtime fault ports | `.evidence/runtime-20260916-130839-499490/postgres-runtime.log` |
| Two actual compiled native fixtures | Passed, including install/enable/grant/invoke/read/replay/exact composition | `.evidence/plugin-runtime/local-flow.json` |
| Actual MinIO adapter, including streaming | Explicit opt-in test passed; not left unexecuted because of the default ignore | `.evidence/runtime-20260916-130124-692880/minio-streaming.log` |
| Frontend typecheck/unit/build | All passed; 47 unit tests in 4 files | `.evidence/runtime-20260916-131729-130727/verification.json` |
| Browser suite | 12 passed: one real OIDC/upload integration plus eleven UI fixture scenarios | `.evidence/runtime-20260916-132803-016951/browser-e2e.log` |
| Local infrastructure/HTTP suite | 60 passed, including 47 HTTP admission/readiness checks | `.evidence/runtime-20260916-133043-109312/local-infrastructure.log` |
| Generated specification/type drift | Both passed | `.evidence/runtime-20260916-132604-355034/verification.json` |
| Working-tree whitespace | `git diff --check` passed | Actual local command result |

The final Rust source/manifests/lock fingerprint is
`sha256:ef55e22f64aee94b0a31d46378de78f6930a986e58b0eb694ba786f0b0b63f0f`.
It remained unchanged during final Rust verification. Other checks have their own
recorded timestamps; the fingerprint is not a production build attestation or a
hash of the browser bundle. Earlier failed check logs remain available beside
corrected runs rather than being relabeled as successful.

## Browser and local service proof

The rebuilt API initially rejected OIDC discovery because Keycloak's dynamic
backchannel metadata advertised internal token/JWKS origins. The local Compose
configuration now publishes one logical issuer origin; the BFF's explicit local
transport override reaches Keycloak internally without weakening issuer or endpoint
validation. The runtime image built successfully, and thirteen local services
were observed running and healthy after the configuration correction.

The real browser probe exercised Authorization Code + PKCE against Keycloak, then
used the verified identity with an explicitly provisioned ephemeral test tenant.
It checked Origin/CSRF, foreign-tenant and step-up rejection before malformed body
parsing, idempotent artifact begin, Confidential upload metadata, pending-content
denial, actual ClamAV scanning and MinIO upload/finalize/readback, immutable upload
rewrite denial and logout. OAuth/session traces and video are disabled. Report:
`.evidence/plugin-runtime/browser-flow.json`.

The other eleven browser cases use deterministic HTTP fixtures for UI behavior:
320/390/768/1440 widths in light/dark themes, accessibility checks, all 23 feature
routes at 320px/200% text, protected deep-link preservation and command receipt
semantics. Those fixture results do not claim real execution of every command.

Local application address: `http://localhost:39850`. Service liveness and successful
local commands do not override incomplete product readiness. Test tenants are
removed; production or user ownership is never inferred from an email or login.
Immutable test object versions remain unreachable pending an object-GC path.

## Cancellation admission checkpoint — 17 September 2026

`run.cancel` now enters the PostgreSQL command dispatcher. This is cancellation
admission, not Temporal cancellation delivery or proof that execution has stopped.
The handler checks the expected version, advances the run fence, and writes the
Run projection, audit, outbox and receipt through the existing transaction path.

- A seeded `QUEUED` run becomes `CANCELLED` with a `SUCCEEDED` command receipt.
- A seeded `RUNNING` run becomes `CANCEL_REQUESTED` with an `ACCEPTED` receipt and
  the same non-null run ID. Neither outcome claims to undo effects.
- Both integration cases verify committed version/fence increments, blocked
  dispatch, identical same-key replay without additional writes, stale-version
  rejection, rejection of a second cancellation with a new key, and preserved
  Confidential projection classification.
- Unit tests cover refusal to directly cancel waiting/blocked runs with unresolved
  effects. The integration cases do not seed a transmitted effect and do not
  establish reconciliation behavior.

Current PostgreSQL evidence: **5 passed, 0 failed**, recorded in
`.evidence/runtime-20260916-233357-245398/verification.json` and its
`postgres-runtime.log`. Source digest before and after the check is
`sha256:5de2147e5bf4aecf8c95f77036cb1a5bf0566a1fd2abab4ee769340e67ba5edb`;
`source_unchanged_during_checks` is true. This supersedes the earlier 4-test
cancellation checkpoint, not the historical full-workspace/browser results above.

An intermediate test run failed because the new RUNNING case still used the
QUEUED case's hard-coded `CANCELLED` assertion. The handler returned
`CANCEL_REQUESTED`; the test refactor was corrected. That failure is not a
red-to-green implementation proof. Production cancellation code was unchanged
while adding the RUNNING integration case.

Independent cancellation review found no confirmed production-code defect in this
admission checkpoint. It recorded future dispatch locking/rechecks, cancellation
cascade and unresolved-effect integration coverage as open obligations, and noted
that Run authorization currently exposes only version/classification rather than
ownership attributes. Its missing-RUNNING-test remark predates the 5-test evidence
above. This is an assistant-spawned review, not named QA or release approval.
No acceptance-driver mappings, Temporal worker, run.start handler, effect
reconciliation, or release qualification are established by this checkpoint.
The immutable requirements baseline is unchanged.

## Temporal SDK cancellation probe — 17 September 2026 (INTERMITTENT FAILURE, SDK compatibility only)

A new opt-in probe
(`platform-plugins/workflow-temporal/tests/cancellation_temporal.rs`, feature
`local-temporal-tests`) runs a real workflow + worker + activity on the existing
`masonwing-dev` Temporal service (127.0.0.1:39854, namespace `masonwing-local`).
It asserts a trace `["sent","reconciled"]` after `handle.cancel` and a detached
`WorkflowCancellationToken` for the reconcile activity. Its trace markers are
in-memory only — it exercises NO PostgreSQL dispatcher, no provider transport
and no real reconciliation. It is a compatibility probe, not product acceptance.

Observed results remain mixed: the probe has both passed repeatedly and timed out
under host contention. **It is not stable or qualified as a check.** Earlier passing
observations follow:

- `cancel-probe-ece3d001…`, `b6e4250f…`, `5ae32b0a…` completed (the probe oracle
  was observed passing in earlier runs before retry bounds were added).
- `cancel-probe-febf4541…`, `72da0502…`, `5d759e23…`, `165a6422…` TimedOut;
  `0af4b83c…` Failed. Structured history of `febf4541` (verified via
  `temporal workflow show --output json`) shows the first workflow task took ~9s
  of its 10s default `workflowTaskTimeout` before the activity was even scheduled;
  after `WorkflowExecutionCancelRequested` the task on the sticky queue hit
  `TIMEOUT_TYPE_START_TO_CLOSE`, and the run closed with
  `WORKFLOW_TASK_FAILED_CAUSE_FORCE_CLOSE_COMMAND` at the execution deadline.
- A later locked `verify_runtime.py temporal` run also failed with a concrete
  activity-level cause: `activity Heartbeat timeout` (3s) with
  `retry_state: MaximumAttemptsReached` on `Probe::sent_then_wait`. Heartbeats at
  100ms are throttled by the SDK core and share a loaded host; under contention
  (a competing `cargo test --workspace` from another project, `steptory.com`,
  blocked this build's package cache for ~1m31s) the 3s heartbeat deadline was
  too tight, and `maximum_attempts(1)` exposed it instead of hiding it.

The revision has both passed and timed out. The workflow uses cancellation detection
(`error.as_cancelled().is_some()`, covering both bare `Cancelled` and
`Failed(...)`-wrapped cancellation per `temporalio-common-wasm-1.0.0/src/error.rs`),
both activities use `maximum_attempts(1)`, `sent_then_wait` uses a bounded 30s
heartbeat timeout (still inside its 60s start-to-close), server-side
`execution_timeout` is 120s with `task_timeout` 30s, and the outer tokio timeout
is 150s. The unused `ActivityExecutionError` import was removed (clippy
`-D warnings` clean). A leaked unbounded run (`a5a65e87`) self-completed before
a terminate-cleanup call landed.

Official evidence: `python3 scripts/verify_runtime.py temporal` → PASS.
`.evidence/runtime-20260917-025520-861153/verification.json`
(`IMPLEMENTATION_VERIFICATION` / `NOT_ACCEPTED`, `source_unchanged_during_checks`
true) and its `temporal-cancellation-probe.log` (1 passed, 0 failed). Historical repeats:
3/3 consecutive repeats passed (28s, 27s, 59s) in
`.evidence/temporal-cancellation-repeat.log`, plus
an earlier isolated PASS in 5.68s (`.evidence/temporal-cancellation-isolated.log`).

Earlier history is retained for honesty: bounded revisions failed with
`TimedOut`/`Elapsed(())` under host contention; widening timeouts alone did not
fix it — the `as_cancelled()` correction plus the heartbeat bound did.

Later evidence retained separately:

- A fresh locked `verify_runtime.py temporal` run passed in
  `.evidence/runtime-20260917-034347-860638/verification.json` (1 passed, 0
  failed, source unchanged).
- Back-to-back manual runs passed at 27.55s and 60.85s with full logs in
  `.evidence/temporal-double-check-N69A3Z/run-1.log` and `run-2.log`.
- A structured failure (before these passes) was captured server-side:
  `cancel-probe-3503c270-bfe2-458b-89de-668004ae90d5` shows the first workflow
  task STARTED then `TIMEOUT_TYPE_START_TO_CLOSE` (task timeout 30s), then
  `WORKFLOW_EXECUTION_TIMED_OUT` at its 120s deadline. No activity was ever
  scheduled and no cancel request was recorded — the run failed before the
  probe's cancellation path was exercised, so that run does not test
  cancellation delivery.

`scripts/verify_runtime.py` `temporal` stage runs this probe with `--locked`.

No independent review of this probe has passed; review remains pending.

## Delivery still pending

Durable Temporal dispatch/checkpoints/replay/cancellation, complete approval/effect
reconciliation and budget reservation/settlement/fairness, qualified external
model/OAuth/MCP providers, remote process transport, automatic drain completion,
plugin migrations, export and object garbage collection remain unfinished or
unqualified. Complete product-specific workflows, production operating gates and
the original acceptance scenarios must still be implemented and exercised.

Implementation references: `registry-invocation-implementation.md`,
`auth-implementation.md`, `artifact-implementation.md` and `web-implementation.md`.
No commit, push, publication, production change or shared-volume deletion was
performed as part of this continuation.
