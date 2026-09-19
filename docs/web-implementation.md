# Web implementation

This document records the implemented Masonwing browser boundary. The immutable requirements and generated contracts remain the authority when this document and a contract differ.

## Session and tenant scope

The React shell reads the baseline `GET /session` `SessionView`. Sign-in navigates to `GET /auth/login?return_to=...` and preserves the complete protected pathname and query string. Sign-out calls `POST /auth/logout` with the current session CSRF token when present.

The browser never stores or accepts bearer tokens. Requests use `credentials: same-origin`. Unsafe command requests carry the session `X-CSRF-Token` and a fresh `Idempotency-Key`.

Tenant data is keyed by tenant, principal and membership epoch through TanStack Query. A tenant switch cancels in-flight tenant reads and removes the previous tenant cache before navigation. Logout clears all authenticated tenant cache. A principal change increments the epoch and clears tenant state. A tenant named in a URL is rejected in the UI before tenant API reads when it is not present in the current session.

## Resource reads

The immutable baseline generic collection contract exposes these resource types:

| Resource type | Main feature surfaces |
| --- | --- |
| `Run` | Workflow |
| `Membership` | Sessions and organization |
| `Delegation` | Agent delegation |
| `EffectIntent` | Effects |
| `Connection` | Connectors |
| `CostReservation` | Budget |
| `PluginManifest` | Composition, registry, UI, marketplace |
| `UploadSession` | Artifacts |
| `ConnectorAuthorization` | Connectors |

Collection reads use `GET /v1/tenants/{tenant_id}/resources/{resource_type}` with cursor pagination. Filtering in the current UI is explicitly limited to resource IDs already loaded; it is not presented as a server search.

The host additionally exposes 19 declared extension views under
`/v1/tenants/{tenant_id}/views/{resource_type}`, with the same projection/artifact
read boundary and cursor semantics. The generated baseline resource enum remains
unchanged. These include `ProductComposition`, `PluginInstallation`,
`PluginInvocation`, `Approval`, `BudgetSettings`, `KillSwitch`, `ProviderProfile`,
`Notification`, `SupportGrant`, `CatalogListing`, `Export`, `ReleaseQualification`,
`ConformanceRun`, `PolicyProposal`, `Schedule`, `DeletionRequest`, `PolicyDecision`,
`MembershipInvitation` and `IdentityConfigurationProposal`. Client tests compare
this declared list with the host's list. Unknown extensions are rejected before
fetching.

Resource details follow the contract projection chain instead of assuming inline JSON:

1. `GET /v1/tenants/{tenant_id}/resources/{resource_type}/{resource_id}` or the declared `/views/` equivalent returns `ResourceProjection`.
2. The projection identifies an `artifact_ref` and the UI verifies the resource identity and artifact tenant.
3. `GET /v1/tenants/{tenant_id}/artifacts/{artifact_id}/content` returns the typed projection content bytes. Current resource projections are decoded as JSON and rendered with type-specific fields.

The normal UI does not expose raw projection JSON. Sensitive implementation fields such as connection `secret_ref` are not rendered.

Feature screens now use the corresponding host view for installed plugins,
invocation results, compositions, proposals and other resource projections. View
renderers select explicit user-facing fields and never dump arbitrary JSON or
secret references. An empty view does not claim its producing command is
implemented. The UI does not manufacture counts, history or results from
requirements metadata.

The status client consumes the implemented BFF `/dev/status` shape, including
PostgreSQL/S3/OIDC/Cedar and `QUALIFICATION_INCOMPLETE`. It rejects responses
claiming enabled external mutations, nonzero live budget or qualified readiness;
it no longer requires the obsolete scaffold-only status shape.

## Command behavior

Feature routes display guided forms when a common command can be represented with the immutable typed request fields. Complex commands requiring a complete `PluginManifest`, reviewed artifacts, grants, effect context or other structured objects remain in the advanced contract catalog with guidance to start from the owning entity or SDK workflow. There is no generic raw-JSON command form.

Each submit creates an idempotency key for one serialized payload. An explicit retry of the same unsent payload reuses that key. Editing the payload creates a new key. A successful receipt closes the attempt, so a later submit is a new command.

Receipt and error states are intentionally distinct:

- `SUCCEEDED` is shown as synchronous command completion.
- `ACCEPTED` is shown as accepted/pending and explicitly states that it is not a final business outcome.
- `409` or `REVIEW_CONFLICT` stops replay and directs the user to refresh/review the current state.
- `UNKNOWN` effect state or `RECONCILE` recovery is shown as an unknown outcome with no blind retry.
- A retry button is only offered for an explicit retryable response whose effect state is `NOT_SENT`, and it preserves the same request/idempotency pair.
- Backend `501`, `NOT_IMPLEMENTED`, `UNSUPPORTED` and equivalent operation-not-found responses are shown as unavailable. No mock success replaces them.

Destructive guided actions require an additional target/state confirmation in the form before submit. Backend authorization remains authoritative regardless of button visibility.

## Feature coverage

All 23 generated Masonwing routes are functional workspace routes. Guided forms currently cover `product.compose`, plugin enable/disable/revoke/uninstall, identity configuration, membership invite/accept/change/revoke, policy evaluation, grant revoke, artifact begin/finalize, run start/cancel, approval decision, effect dispatch/reconcile, kill switch, budget configure, provider compact, connection authorize/revoke, dead-letter replay, UI register, notification read, support request and conformance run.

The advanced catalog keeps the remaining screen operations visible without pretending a safe generic editor exists, including plugin install/upgrade/invoke, policy proposal, grant creation, deletion requests, effect proposal/compensation, provider configuration, schedule creation, export creation, release qualification and catalog submit/review.

## Navigation and accessibility

The desktop shell keeps the 240px navigation rail; below 768px it uses the shared dialog-based navigation sheet. The tenant selector, theme control and logout action remain in the global header. Every route has one `h1`.

Resource selection is stored in the URL as `?type={resource_type}&resource={resource_id}`. This supports direct links and browser Back/Forward. Sign-in preserves that query in `return_to`. Changing tenant remounts the feature surface so tenant-specific draft form state is not carried into a different tenant.

Controls use the shared minimum 44px interaction sizing. Resource lists use semantic lists containing buttons. Loading, empty, filtered-empty, error, offline, stale, forbidden, unsupported, pending, conflict and unknown-outcome states have textual labels in addition to color. The styles support light/dark themes, reduced motion and the baseline 320px/200% text target.

## Verification

Focused browser-unit coverage is in:

- `tests/web/behavior.test.tsx`: contribution boundary, tenant cache cancellation and transport/failure semantics.
- `tests/web/routes.test.tsx`: all 23 routes, auth deep links, unauthorized tenant denial, projection-to-artifact detail reads, unavailable backend state and tenant switching.
- `tests/web/commands.test.tsx`: idempotency reuse/new-key behavior, conflict review and unknown-outcome no-retry behavior.
- `tests/web/runtime-views.test.ts`: current runtime status, host-view inventory, resources/views routing and tenant-bound projection reads.

Playwright coverage is in `tests/web/e2e/scaffold.spec.ts`. It uses deterministic HTTP fixtures for UI-state evidence while exercising the real built React application at the configured base URL `http://localhost:39850`. It covers 320/390/768/1440 light/dark, serious/critical axe checks, the mobile navigation focus return, all 23 routes at 320px with 200% text, protected deep-link login and command proof headers/`ACCEPTED` presentation. These fixture runs are UI evidence, not backend product acceptance.

The integrated Playwright run must be executed against the refreshed local web build/Compose owned by the prime integration flow. Backend acceptance remains separate and must use real adapters and observable side-effect evidence required by the immutable test suite.

`tests/web/e2e/runtime-auth.spec.ts` additionally passed against real local
Keycloak, BFF, PostgreSQL, MinIO and ClamAV. It covers Authorization Code + PKCE
login, current tenant membership, cookie/session behavior, Origin/CSRF, tenant
and step-up denial before body parsing, idempotent upload begin, confidential
metadata, pending-content denial, scanned upload/finalize/readback and logout.
Only an ephemeral operator-provisioned tenant is used. Traces/video are disabled
to avoid retaining OAuth/session material. The report is
`.evidence/plugin-runtime/browser-flow.json`; this is browser/application
integration, not full product acceptance.
