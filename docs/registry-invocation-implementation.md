# Registry, composition and synchronous plugin execution

Implementation update: 16 September 2026. The immutable requirements, generated
contracts and applied migrations retain their original identities and thresholds.
This report describes application behavior and its verification scope, not full
product acceptance or release approval.

## Application behavior

`product.compose`, `plugin.install`, `plugin.enable`, `plugin.disable`,
`plugin.upgrade`, `plugin.revoke`, `plugin.uninstall` and `plugin.invoke` reach
PostgreSQL handlers through the current-authority, tenant-lock, optimistic-version
and idempotency boundary. Each successful change persists its resource projection,
audit/outbox records and command receipt in the same database transaction.

Installation checks the immutable manifest schema, host-registered capabilities,
contract versions, handler/schema bindings, UI namespaces, the complete selected
dependency graph, a trusted publisher signature and actual package/SBOM/schema
bytes. Cycles, missing required dependencies, incompatible contract ranges and
reverse-dependency-breaking upgrades are denied before registry changes. A plugin
cannot register a capability by requesting it. Initial state is
`INSTALLED_DISABLED`, with no grants.

Enable validates every requested grant against both the signed manifest and
current authorization, and requires enabled dependencies. Upgrade appends an
immutable version, preserves older records for pinned runs and never broadens
grants. Disable records `DRAINING` before `DISABLED`; active runs retain the
draining state. A later explicit disable can complete draining once runs finish.
Digest-specific revocation fences matching unfinished runs; revoking an older
digest does not disable an unrelated newer digest. Uninstall retains evidence and
requires no active runs or required dependants.

Nonempty migrations fail with `PLUGIN_MIGRATION_UNQUALIFIED` before execution.
`EXPORT_THEN_RETAIN` fails with `EXPORT_REQUIRED` until export is implemented.
Composition is create-only because its immutable command has no expected-version
field; another lock cannot silently replace an existing composition. Automatic
drain completion and workflow execution are separate, unfinished work.

## Signature and exact lock formats

Publisher keys and signatures are operator-owned PostgreSQL records. The runtime
has read access; there is no HTTP endpoint that lets a submitted manifest create
its own trust record. Native code additionally requires a compiled
`(plugin_id, artifact_digest)` allowlist entry.

`manifest_signing_message` produces a canonical JSON (JCS) envelope:

```json
{
  "purpose": "masonwing.plugin-manifest.v1",
  "publisher_id": "the-publisher-id",
  "artifact_digest": "sha256:<actual-package-hash>",
  "manifest_digest": "sha256:<canonical-manifest-without-signature_ref>"
}
```

Verification uses Ed25519 strict verification against the current unrevoked key.
The signature lookup handle is excluded to avoid a circular content identity.
Artifact digests always hash actual stored bytes.

The baseline's opaque `plugin_lock_ref` addresses this strict host extension:

```json
{
  "schema_version": "1.0.0",
  "product_id": "a-product-id",
  "plugins": [
    {
      "plugin_id": "a-plugin-id",
      "artifact_digest": "sha256:<exact-installed-package-hash>",
      "contract_version": "1.0.0"
    }
  ]
}
```

All selected plugins must already be enabled at those exact digests; required
dependencies must be included. IDs are unique, unknown fields are rejected and
dependency ordering is deterministic. No generated baseline contract was changed.

## Invocation admission and result persistence

`plugin.invoke` requires a current delegation with `plugin.invoke`,
`artifact.read`, the exact installation ResourceRef and the exact input Artifact
ResourceRef. Raw artifacts are grant resources using their real active metadata
version. Handler capabilities are intersected with installation grants, delegation
actions and current Cedar authorization. Cross-tenant, out-of-scope, expired,
revoked, draining and kill-switched requests cannot reach guest code.

The input must be readable and have active, unexpired `allow_transform` rights.
The host verifies actual bytes, validates the JSON instance against the signed
schema and canonicalizes it for the SDK boundary. `PurePluginRuntime` receives
typed contracts and bytes; it has no SQL, storage or secret handle. The concrete
`HostPluginRuntime` supplies only `DenyEffects`. `PROPOSES_EFFECT` handlers fail
with `EFFECT_BROKER_REQUIRED` until a durable effect broker is connected.

After execution, grant/source-right expiry is checked again and the actual output
must pass the signed output schema before persistence. Output artifacts inherit
input classification and rights. Metadata projections inherit the strongest
referenced artifact classification. Both output and projection participate in
artifact lineage. Replay returns the persisted receipt without another handler
invocation.

UploadSession contracts carry a plain artifact ID, so the host attaches explicit
source provenance when creating their projections. Classification comes from the
authoritative artifact row even while it is `PENDING`, and lineage is retained.
A pending Confidential upload therefore does not expose Internal-classified
metadata or lose its relationship to future deletion requests.

The synchronous result is a `PluginInvocation` extension projection at
`/v1/tenants/{tenant_id}/views/PluginInvocation`, wrapped in the existing immutable
ResourceProjection contract. It is not an asynchronous Temporal Run.

Detected corruption carries the exact failed immutable reference across rollback.
A separate reauthorized transaction rereads the object and quarantines it only
when corruption is proven, with audit/outbox records. A concurrent upgrade cannot
redirect that containment to another package. Rejected business changes remain
rolled back.

Native registrations retain reviewed handler IDs, schema content digests,
classification/version, effects, capabilities and deadlines. Tenant-local UUID
remapping is allowed; rebinding compiled code to a different handler contract is
not. The two compiled fixture registrations are enabled only in the local
composition root.

## Execution limits and qualification boundaries

Wasmtime components have no WASI or ambient filesystem/network/environment/secret
access. Guest limits include 128 MiB linear memory, fuel and a five-second guest
deadline. The synchronous host admits at most one blocking compilation/execution
job; its permit remains attached to a timed-out job, preventing an unbounded
queue of abandoned jobs. Late pure results cannot write artifacts or effects.

Fuel/epoch interruption cannot preempt an arbitrary synchronous Rust callback.
This path avoids host I/O callbacks by denying effects. General effect-capable
brokers still need independent I/O deadlines, cancellation and late-write safety.
Guest memory limits are not a bound on all JIT/compiler/host memory, and caller
timeout does not cancel compilation. The separate runner service has no execution
transport yet; the application calls the runner library.

Payload schemas use a bounded offline Draft 2020-12 profile: 256 KiB schema size,
bounded structure/depth/pattern length and fragment-only references. External/file
references and dynamic/recursive-reference keywords are rejected. Unsupported
profiles fail installation rather than bypassing validation. Diagnostics omit
submitted secret values.

## Reproduction and observed evidence

The SDK fixtures contain real canonical schema and WorkflowDefinition artifacts.
`materialize_fixture` replaces descriptive artifact handles with tenant UUIDs,
updates references and recalculates workflow identity. `WorkflowDefinition.digest`
hashes the canonical definition excluding its own digest field; ArtifactRef hashes
the complete stored definition including that field.

```sh
cargo test --locked --workspace --all-targets --jobs 2
cargo test --locked -p masonwing-data-postgres --features local-postgres-tests --test registry_postgres --jobs 2
cargo run --locked -p masonwing-host-api --example local_plugin_flow --jobs 2 -- --local
python3 scripts/verify_runtime.py contracts rust postgres native scaffold auth web
```

The native example uses fixed loopback endpoints in `masonwing-dev`, creates a new
ephemeral tenant and stages synthetic operator-owned trust/artifacts. It then uses
the runtime application role for install, enable, delegate, invoke, read and exact
composition. It verifies actual expected outputs, Confidential classification and
receipt replay. Database records are removed afterward. Immutable MinIO test
objects remain unreachable pending a future object-GC path; no shared volume is
reset.

`.evidence/plugin-runtime/local-flow.json` records the observed successful run:
both native handlers produced verified output through real PostgreSQL and MinIO,
with exact composition and no model calls or external provider mutations. The
actor is an operator-created synthetic identity. This is not proof of browser/OIDC,
scanner-upload, Temporal, remote-worker or full product acceptance.

Separately, `.evidence/plugin-runtime/browser-flow.json` records a passing actual
browser flow through local Keycloak, BFF, PostgreSQL, MinIO and ClamAV. It verifies
Authorization Code + PKCE, session/Origin/CSRF, current tenant and step-up denial
before request-body parsing, idempotent confidential upload metadata, actual
malware scanning, finalize/readback, immutable-write denial and logout. The test
creates/removes its own ephemeral tenant and captures no OAuth/session traces.
This does not qualify the unfinished Temporal or external-provider paths.

The PostgreSQL fault suite additionally verifies denied capabilities/rights with
zero handler calls, invalid-output rollback, optimistic races, idempotency, stale
locks, revoked authority, tampered-package quarantine and confidential output/
projection lineage. It caught and fixed a field-count bug in ArtifactRef detection
that had downgraded projection classification.

`scripts/verify_runtime.py` records actual argv, exit codes, logs and a fingerprint
of Rust sources/Cargo manifests and lockfile. Evidence distinguishes failed runs
from corrected runs and detects source changes during checks.

## Remaining delivery work

Acceptance drivers remain incomplete; these checks do not turn pending AC/TC
scenarios into PASS. Remaining work includes durable Temporal dispatch/checkpoints/
replay/cancellation, approvals/effect reconciliation/budget settlement/fairness,
qualified model/OAuth/MCP providers, remote transport, automatic drain/migration/
export/garbage collection, complete product-driven UI contribution routing,
Gleanbird workflows, browser authentication qualification and operational/release
gates. Readiness remains `QUALIFICATION_INCOMPLETE`.
