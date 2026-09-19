# Auth implementation (WP-005–WP-008 boundary)

This document describes the implemented Masonwing OIDC BFF, SQL-backed browser
session authority and Cedar adapter. The immutable requirements remain
`masonwing-requirements-v1.0.1`; this file is implementation guidance and test
status, not a replacement specification.

## Components

| Component | Owned implementation | Responsibility |
| --- | --- | --- |
| OIDC | `platform-plugins/identity-oidc` | Provider trust, Authorization Code + S256 PKCE, one-use state/nonce transaction, token exchange/ID-token verification, issuer+subject identity |
| Session/current membership | `crates/host-api/src/auth/session.rs` | `tower-sessions` PostgreSQL session, secure cookie profile, idle/absolute expiry, CSRF+Origin, membership/permission epochs |
| Cedar | `platform-plugins/authorization-cedar` | Default deny, forbid precedence, diagnostic fail-closed, host-built entities |
| Gate | `crates/host-api/src/auth/gate.rs` | Route tenant + current ACTIVE membership + current Cedar policy + optional step-up |
| HTTP | `crates/host-api/src/auth/router.rs` | `/auth/login`, `/auth/callback`, `/auth/logout`, `/auth/backchannel-logout`, `/session` |
| Persistence | `infra/migrations/0002_identity_runtime.sql` | OIDC identities/transactions, membership identity+epoch additions, current Cedar policy versions, tower-session table |

## OIDC trust and login

`OidcIdentityAdapter::discover` validates the configured logical issuer before
network use. The discovery document's `issuer`, authorization endpoint, token
endpoint and JWKS URI must all belong to the logical issuer origin. Redirects are
disabled on the OIDC HTTP client. JWKS is fetched only after those checks pass.
The ID-token verifier is additionally restricted to the configured signing
algorithm allowlist; the local Keycloak profile defaults to RS256.

`begin_login` validates `return_to` before inserting any auth transaction.
Targets must be an internal absolute-path reference beginning with one `/`;
external/scheme-relative targets, backslashes, control characters, CRLF and
percent-encoded slash/backslash/CRLF forms are rejected. The adapter creates
PKCE S256 challenge/verifier plus random state, nonce and browser binding. The
server stores state, nonce, verifier, binding, issuer, return target and a five
minute expiry in `oidc_auth_transactions`. OAuth tokens are never stored there.

The host stores only the random browser binding in the anonymous server-side
session. Callback completion performs `DELETE ... RETURNING` with state + browser
binding + issuer + expiry before token exchange. Therefore concurrent callback
replays cannot both obtain the nonce/verifier. Provider errors consume a valid
transaction but do not create an application session. A successful callback
verifies the ID token and nonce with `openidconnect`, checks `at_hash` when
present, upserts identity by `(issuer, subject)`, clears anonymous state and
cycles the tower session ID across the login privilege boundary.

Email is metadata only. `oidc_identities` has a unique `(issuer, subject)` key,
and a database trigger prevents the identity key/principal ID from being changed
in place. Two providers returning the same email remain distinct principals.

### Local Compose transport

The local fixture advertises the browser-visible issuer
`http://localhost:39853/realms/masonwing`, while a backend container reaches
Keycloak through `http://keycloak:8080`. `OidcProviderConfig.internal_base_url`
supports that split only when `allow_loopback_http=true` and the logical issuer
is HTTP loopback. Discovery/JWKS/token fetches rewrite only the transport origin;
the discovery metadata must still declare the logical issuer and logical
same-origin endpoints, browser authorization remains on the logical endpoint,
and ID-token `iss` verification remains the logical issuer. Non-loopback
configuration cannot enable this override.

Local Compose now sets `KC_HOSTNAME_BACKCHANNEL_DYNAMIC=false`. The observed
dynamic setting advertised internal token/JWKS origins for internal discovery,
which the adapter correctly rejected. Publishing one logical endpoint origin
keeps trust checks intact; only the BFF transport is rewritten. The actual local
browser login/upload test passed with Keycloak, session cookies, tenant admission
and CSRF. Evidence: `.evidence/plugin-runtime/browser-flow.json`. HTTP-only local
cookie behavior is not production cookie qualification.

Production/default configuration must use HTTPS and leave both
`allow_loopback_http=false` and `internal_base_url=None` unless a separately
qualified transport design is introduced.

## Browser session and cookies

The host uses `tower-sessions 0.14` with
`tower-sessions-sqlx-store 0.15`/PostgreSQL. Migration 0002 creates the store's
exact `tower_sessions.session(id, data, expiry_date)` shape so the runtime does
not need DDL privileges.

`SessionCookieProfile::Secure` is the default production profile:

- cookie name `__Host-masonwing_session`
- `Secure`, `HttpOnly`, `Path=/`, no Domain
- `SameSite=Lax`
- 30 minute inactivity expiry

`SessionCookieProfile::LoopbackDev` is explicit and changes only the properties
required for plain-HTTP localhost testing: the cookie is named
`masonwing_session` and `Secure=false`. It must not be selected for a live
environment.

The application envelope contains principal UUID, issuer, subject, CSRF token,
creation/last-seen timestamps, 12-hour absolute expiry, optional step-up expiry
and active tenant hint. It contains no access/refresh/ID token bytes. Request-time
checks enforce both idle and absolute expiry, so an expired record cannot be
revived even if periodic cleanup has not run yet. `cleanup_expired_sessions`
delegates physical expiry cleanup to the SQLx session store.

`POST /auth/logout` reads the session without extending it, validates exact
configured `Origin` and `X-CSRF-Token`, then calls `Session::flush`. This preserves
the required ordering: an unsafe forged request cannot mutate the inactivity
timestamp before CSRF validation.

Other unsafe cookie-authenticated command routes must preserve the same ordering:
`peek_authenticated` → exact Origin/CSRF verification →
`authorize_current`/`authorize_current_and_nested` → body/command handling. The
authorization gate touches the inactivity timestamp, so calling it before CSRF
would violate the mutation-ordering requirement.

## Membership/current authority

Migration 0002 adds `membership_id` and `permission_epoch` to `memberships`.
Role or status changes trigger a monotonic increase of both `membership_epoch`
and `permission_epoch`. Existing rows initialize permission epoch from their
membership epoch without an RLS-sensitive `UPDATE`: PostgreSQL first materializes
`permission_epoch` as a stored generated copy of `membership_epoch`, then the
migration drops the generated expression and installs the mutable default. The
membership UUID is likewise populated by its `ADD COLUMN ... DEFAULT` operation.
The existing tenant FORCE-RLS policy remains in force throughout; migration 0002
does not disable or bypass RLS for backfill.
Additional SELECT policies permit the host to list only the authenticated
principal's active memberships after it sets transaction-local
`app.principal_id`; mutation checks remain tenant scoped.

`SessionAuthority::current_membership` and `current_authority` start a SQLx
transaction, set transaction-local `app.tenant_id`, then re-read ACTIVE
membership (and ACTIVE tenant). `current_authority` also reads the one current
Cedar policy version in the same transaction. These values become the gate's
`VerifiedActor` and `CurrentPermit`:

```text
VerifiedActor
  tenant_id: UUID
  principal_id: UUID
  issuer / subject
  membership_id: UUID
  role
  membership_epoch: i64
  permission_epoch: i64
  policy_version: semver string
  policy_epoch: i64
  step_up_expires_at: optional timestamp

CurrentPermit
  actor: VerifiedActor
  policy_version: semver string
  policy_epoch: i64
```

The command transaction runtime must still re-lock/re-read membership epochs and
the current policy version/epoch immediately before a write/dispatch. The auth
gate provides the values to compare; it does not replace the write transaction's
point-of-no-return check.

`AuthGate::authorize_current_and_nested` evaluates an outer action plus every
nested action/resource against one current membership/policy snapshot. Grant and
effect handlers must use this path for requested delegated/dispatch capabilities:
authorization for `grant.create` or `effect.propose` does not authorize a broader
inner action. The returned `CurrentPermit` repeats the same policy version/epoch
embedded in its `VerifiedActor`, giving the command transaction an explicit
binding to recheck.

`GET /session` emits the exact baseline `SessionView` fields only:
`authenticated`, `principal`, `tenant_ids`, `active_tenant_id`, `expires_at`,
`step_up_expires_at`, `csrf_token`.

## Cedar adapter

`CedarAuthorizationAdapter::evaluate` takes an immutable parsed
`CedarPolicySnapshot` and a `TrustedAuthorizationRequest`. The latter is intended
to be constructed by host code from database/route facts; the HTTP/plugin input
surface must not deserialize a raw Cedar entity bag.

The adapter builds the principal and resource entities itself. Cedar supplies
default-deny and forbid-overrides-permit semantics. Masonwing adds a stricter
check after evaluation: if `response.diagnostics().errors()` contains any entry,
the adapter returns `AUTHZ_EVALUATION_ERROR` regardless of Cedar's decision.
Principal/resource tenant mismatch is rejected before Cedar evaluation. A host
resource whose trusted tenant differs from the route tenant maps to the generic
404 shape so cross-tenant existence is not disclosed.

Policy rows are tenant FORCE-RLS protected. `(tenant_id, policy_version)` and
`(tenant_id, policy_epoch)` are unique, only one row may be current, and a trigger
prevents source/version/epoch from being rewritten after publication.

## Step-up

Sensitive gate calls set `AuthorizationInput.requires_step_up=true`. They fail
closed unless the session contains a non-expired step-up timestamp. The existing
OIDC route starts the stronger flow with `GET /auth/login?step_up=true`: the BFF
requires an authenticated session, sends Authorization Code + PKCE with
`prompt=login`, `max_age` and the configured ACR request, then stores the exact
initiating principal/issuer/subject as server-side flow purpose.

The callback resolves the returned issuer+subject only against that existing
principal; it cannot create or email-link a different account. After normal ID
token signature/issuer/audience/nonce checks, `verify_step_up` additionally
requires a fresh `auth_time`, an exact configured ACR and at least one configured
strong AMR such as OTP or WebAuthn. `pwd`/password by itself is deliberately not
in the strong-AMR allowlist and therefore cannot mint step-up. Successful proof
cycles the session ID and creates a five-minute step-up window.

The synthetic local Keycloak realm is password-only today, so it is expected to
fail step-up until an MFA authentication flow that emits the configured ACR/AMR
is added and qualified. This is intentional fail-closed behavior; local password
login must not be treated as evidence for sensitive operations.

## Deliberately unqualified behavior

`POST /auth/backchannel-logout` currently returns controlled 503
`AUTH_BACKCHANNEL_NOT_QUALIFIED`. REQ-038 requires verified signature, issuer,
audience, events, sid/sub, jti replay protection and idempotence; none of those is
substituted with a partial parser.

Refresh tokens are not persisted or used in this slice, so refresh-token rotation
is not claimed qualified. Local logout invalidates the Masonwing session
immediately; an IdP end-session follow-up is not yet implemented. Password and
implicit grants have no application route and the local Keycloak client keeps
both disabled.

## Host integration

Prime host integration needs these direct dependencies in
`crates/host-api/Cargo.toml` in addition to existing host dependencies:

```toml
chrono.workspace = true
sqlx.workspace = true
uuid.workspace = true
serde_json.workspace = true
masonwing-identity-oidc = { path = "../../platform-plugins/identity-oidc" }
masonwing-authorization-cedar = { path = "../../platform-plugins/authorization-cedar" }
tower-sessions = "0.14.0"
tower-sessions-sqlx-store = { version = "0.15.0", features = ["postgres"] }
time = { version = "0.3", features = ["serde"] }
```

The crate must expose `pub mod auth;`. Build `SessionAuthority` from the common
`PgPool`, construct `OidcIdentityAdapter`, construct `AuthState`, merge
`auth::router(state)`, then apply `SessionAuthority::session_layer(...)` outside
that router. The local config maps the existing Compose values as follows:

```text
issuer              = OIDC_ISSUER
internal_base_url   = OIDC_INTERNAL_BASE_URL
client_id           = masonwing-local-bff
client_secret       = synthetic local client secret/reference
redirect_uri        = OIDC_REDIRECT_URI
allow_loopback_http = true only for LOCAL
cookie profile      = LoopbackDev only for LOCAL HTTP
expected Origin     = http://localhost:39850
```

Callback access logging must omit/redact the query string because it carries the
authorization code and state. Do not add `Uri`/raw query logging middleware for
`/auth/callback`.

## Current verification evidence

Focused crate tests currently cover:

- OIDC local-return validation and open-redirect rejection
- logical issuer/endpoint trust and explicit loopback transport rewrite
- provider error sanitization
- Cedar default deny
- Cedar forbid overriding a matching permit
- fail-closed Cedar diagnostics even when another permit matches
- tenant-context mismatch before Cedar evaluation
- step-up denial for password-only, stale authentication and wrong ACR
- step-up evidence only for fresh matching ACR plus configured strong AMR
- schema structural checks in `tests/auth/test_identity_runtime_contract.py`

The rollback-only PostgreSQL runtime probe also passed against the migrated
`masonwing-dev` volume using the non-owner `masonwing_app` role. It verified
FORCE-RLS principal scope, monotonic membership/permission epoch bump, distinct
issuer/sub identities despite equal email metadata, one-use OIDC state
consumption, tenant-scoped current Cedar policy visibility and tower-session
CRUD; the transaction was rolled back after the assertions.

Migration 0002 has been applied through the Masonwing migration CLI and its
checksum verified. The host auth module compiles in the actual host crate and its
focused CSRF/cookie/nested-authority tests pass. Browser-level Keycloak login is
still separate integration evidence; the synthetic local IdP does not yet
qualify MFA step-up, as described above.
