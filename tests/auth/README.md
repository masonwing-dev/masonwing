# Auth test scope

`tests/auth/test_identity_runtime_contract.py` is a bounded schema/ownership test
for migration `0002_identity_runtime.sql`. It is not an acceptance substitute.

Executable protocol/policy behavior is covered in the owned Rust crates:

- `masonwing-identity-oidc`: return target rejection, issuer/endpoint trust,
  explicit loopback transport and provider-error sanitization.
- `masonwing-authorization-cedar`: default deny, forbid precedence, diagnostic
  fail-closed behavior and tenant-context rejection.
- `masonwing-host-api::auth` (once prime wires the module/dependencies): exact
  Origin + synchronizer-CSRF checks plus session/gate behavior.

Local PostgreSQL/Keycloak qualification must run only after migration 0002 is
applied by the Masonwing migration owner and the host auth router is integrated.
Until that evidence exists, back-channel logout and refresh-token behavior remain
unqualified.

`postgres_runtime.sql` is a rollback-only runtime-role integration probe. Run it
through `psql` as the local migration/admin fixture after the numbered migration
CLI has completed. It switches to `masonwing_app` inside one transaction and
checks FORCE-RLS principal scope, authority-epoch bumps, issuer/sub identity
separation, one-use OIDC transaction consumption, tenant-scoped current policy
and tower-session storage. It always rolls back on success.
