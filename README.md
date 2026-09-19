# Masonwing

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.97.1-orange.svg)](rust-toolchain.toml)

**Masonwing** is an open-source, domain-neutral agent runtime and plugin host written in Rust with a React developer shell and pluggable infrastructure adapters.

Masonwing is being implemented against immutable requirements for fail-closed authorization, multi-tenant isolation, immutable approvals, bounded execution and explicit reconciliation of uncertain external outcomes. The complete product is not yet qualified for production.

## Current implementation

The application includes OIDC/session and Cedar boundaries, tenant-scoped PostgreSQL command transactions, an S3 artifact lifecycle, plugin registry/composition/lifecycle commands and synchronous `plugin.invoke`. Native dispatch uses a compiled package allowlist; Wasmtime components have no WASI access. Payloads are checked against signed schema artifacts, with current grants, source rights, immutable receipts and audit/outbox persistence.

Both checksum and document-review fixtures have executed through the same host runtime and real local PostgreSQL/MinIO storage. Their results retain input classification, and replay returns the original receipt. See [registry and invocation](docs/registry-invocation-implementation.md), [auth](docs/auth-implementation.md), [artifacts](docs/artifact-implementation.md) and [web shell](docs/web-implementation.md) for scope and evidence.

The [16 September runtime verification report](docs/runtime-verification-20260916.md) records exact test results, real browser/Keycloak/upload evidence, source fingerprints and remaining delivery work.

Durable Temporal orchestration, complete approval/effect/budget settlement and provider dispatch, remote execution, full Gleanbird workflows, automatic plugin migrations and production acceptance still require implementation and qualification. Readiness remains 503; external provider mutations and live budget remain disabled. Local successful commands are not release acceptance.

---

## Required Invariants & Architectural Principles

1. **Domain-Neutral Core**: The kernel contains zero business or vertical-specific logic. Vertical agent products consume the public SDK (`masonwing-sdk`) without modifying the host platform.
2. **Fail-Closed Tenancy & Permissions**: Tenant boundaries and fine-grained authorization are enforced before parsing untrusted payload bodies. Child grants cannot exceed parent capabilities, and tokens expire at exact boundaries.
3. **Resilient Side-Effect Broker**: When external mutation calls encounter ambiguous network timeouts, the broker records `OUTCOME_UNKNOWN` and schedules explicit reconciliation. Blind retries are strictly prohibited.
4. **Bounded Agent Execution**: Bounded loops enforce strict execution turn limits, preventing recursive runaway agent invocations and infinite spend.
5. **Immutable Approval Bindings**: Approvals are bound to exact target revisions, content digests, scopes, and expiration timestamps. Any modified content digest immediately invalidates prior approvals.
6. **Conservative Budget Retention**: Usage that cannot be confirmed immediately retains its full budget reservation until reconciled, avoiding falsified zero-cost assumptions.

---

## Workspace Structure

```text
crates/
├── contracts/             # Wire schemas, opaque identifiers, and operation catalog
├── kernel/                # Host-side invariants, state machines, guards, and ports
├── sdk/                   # Public domain-plugin SDK (host broker, state transitions)
├── host-api/              # Axum HTTP service with fail-closed command routing
├── worker/                # Worker boundary; Temporal task consumption pending
├── component-runner/      # Wasmtime library + health service; transport pending
├── remote-runner/         # Remote execution boundary, currently unavailable
└── cli/                   # Developer CLI (masonwing status, version)

platform-plugins/          # 10 pluggable infrastructure adapter boundaries
├── identity-oidc/         # OpenID Connect / Keycloak authentication adapter
├── authorization-cedar/   # Cedar policy evaluation engine adapter
├── data-postgres/         # PostgreSQL persistence with Row-Level Security (RLS)
├── workflow-temporal/     # Temporal durable workflow engine adapter
├── model-provider/        # LLM provider adapter with compaction
├── connections/           # OAuth connectors and MCP protocol brokers
├── artifacts/             # Content-addressable object storage (S3) adapter
├── budget/                # Quota and budget management adapter
├── events/                # Event bus and replay adapter
└── secrets/               # Secret vault (OpenBao) adapter

products/                  # Domain-plugin test fixtures
├── checksum/              # Deterministic, no-model test fixture
└── document-review/       # Second-domain SDK conformance fixture

packages/                  # Web workspace (pnpm + React + Vite + TypeScript)
├── web-shell/             # Developer portal and feature catalog shell
├── ui/                    # Shared component library and design system tokens
├── plugin-sdk/            # Browser-side plugin contribution registry
└── api-client/            # Typed client generated from OpenAPI schemas

infra/                     # Local infrastructure recipes and Compose definitions
contracts/                 # JSON Schemas, OpenAPI specifications, and context maps
tests/                     # Scaffold, integration, unit, and Playwright E2E suites
scripts/                   # Code generation, synchronization, and verification tools
docs/                      # Architecture, DDD boundaries, and TDD documentation
```

---

## Quickstart

### Prerequisites

- **Rust**: `1.97.1` (configured via `rust-toolchain.toml`)
- **Node.js**: `>= 22.12.0` and **pnpm** `11.24.0` (see `package.json`)
- **Python**: `>= 3.11`
- **Docker** & **Docker Compose**

### 1. Bootstrap and Launch Local Stack

```sh
make bootstrap
make dev-up
make dev-status
```

All local development services bind exclusively to loopback (`127.0.0.1`) on reserved ports `39850–39859` through an isolated HAProxy gateway:

| Port | Service | Description |
|---|---|---|
| `39850` | Web Shell | React developer shell & feature inspection |
| `39851` | Rust BFF | Core API gateway (`/health/live`, `/dev/status`) |
| `39852` | PostgreSQL | Application database with multi-tenant RLS |
| `39853` | Keycloak | Identity provider (OIDC PKCE realm) |
| `39854` | Temporal gRPC | Durable workflow engine |
| `39855` | Temporal UI | Workflow execution inspector |
| `39856` | S3 API | Versioned artifact storage |
| `39857` | Storage Console | Local object storage browser |
| `39858` | OpenBao UI | Secret management vault |
| `39859` | Provider Fixtures| Synthetic fault-injection test harness |

### 2. Run Verification and Tests

```sh
# Lint and type check
make check

# Run Rust unit and invariant tests
make test

# Run integration tests against local Docker stack
make test-integration

# Run browser end-to-end tests
pnpm exec playwright install chromium
make test-e2e

# Application checks with actual local PostgreSQL and native fixture execution
python3 scripts/verify_runtime.py contracts rust postgres native scaffold auth web
```

---

## Test-Driven Development (TDD)

Masonwing enforces specification integrity via test-driven development:

```sh
# Synchronize schemas and verify zero drift
python3 scripts/sync_specs.py --check
node scripts/generate-types.mjs --check

# Run acceptance tests
.venv/bin/python -m pytest tests/acceptance -q
```

Unimplemented business handlers deliberately remain `NOT_IMPLEMENTED` (RED) until their corresponding scenario drivers and qualified adapters are implemented. No synthetic mock receipts are permitted to mask incomplete behavior.

---

## Health Semantics

- `GET /health/live`: Returns `200 OK` when the process is actively serving.
- `GET /health/ready`: Returns `503 Service Unavailable` until all critical write adapters are qualified against production requirements.
- `GET /dev/status`: Reports local runtime configuration and incomplete qualification, with external provider mutations disabled and live budget set to zero.

---

## License

Masonwing is licensed under the [Apache License, Version 2.0](LICENSE).
