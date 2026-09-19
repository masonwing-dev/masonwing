\set ON_ERROR_STOP on

-- Rollback-only integration probe for MASONWING@1.0.1 WP-005..WP-007.
-- Run against the local masonwing-dev database after numbered migrations have
-- been applied. It intentionally uses the runtime role for all authority-table
-- operations and leaves no persistent rows.

BEGIN;

SELECT tenant_id::text AS seed_tenant_id,
       principal_id::text AS seed_principal_id,
       membership_epoch::text AS seed_membership_epoch,
       permission_epoch::text AS seed_permission_epoch
FROM memberships
WHERE status = 'ACTIVE'
ORDER BY tenant_id, principal_id
LIMIT 1
\gset

SET LOCAL ROLE masonwing_app;

-- FORCE RLS must hide memberships when neither tenant nor principal context is
-- present.
SELECT count(*) = 0 AS no_context_hidden
FROM memberships
\gset
\if :no_context_hidden
\else
  \echo 'FAIL: memberships visible without runtime authority context'
  \quit 1
\endif

-- `/session` gets only the authenticated principal's memberships by setting a
-- transaction-local principal context; it does not disable FORCE RLS.
SELECT set_config('app.principal_id', :'seed_principal_id', true);
SELECT count(*) = 1 AS principal_self_visible
FROM memberships
WHERE principal_id = :'seed_principal_id'::uuid
  AND tenant_id = :'seed_tenant_id'::uuid
\gset
\if :principal_self_visible
\else
  \echo 'FAIL: principal self-scope membership is not visible'
  \quit 1
\endif

SELECT count(*) = 1 AS principal_tenant_visible
FROM tenants
WHERE id = :'seed_tenant_id'::uuid
  AND status = 'ACTIVE'
\gset
\if :principal_tenant_visible
\else
  \echo 'FAIL: active tenant is not visible through principal self-scope'
  \quit 1
\endif

-- Tenant-authorized membership mutation must monotonically advance both epochs.
SELECT set_config('app.tenant_id', :'seed_tenant_id', true);
UPDATE memberships
SET role = CASE WHEN role = 'VIEWER' THEN 'EDITOR' ELSE 'VIEWER' END
WHERE tenant_id = :'seed_tenant_id'::uuid
  AND principal_id = :'seed_principal_id'::uuid;

SELECT membership_epoch = :'seed_membership_epoch'::bigint + 1
       AND permission_epoch = :'seed_permission_epoch'::bigint + 1 AS epochs_bumped
FROM memberships
WHERE tenant_id = :'seed_tenant_id'::uuid
  AND principal_id = :'seed_principal_id'::uuid
\gset
\if :epochs_bumped
\else
  \echo 'FAIL: membership authority epochs did not advance together'
  \quit 1
\endif

-- Equal email metadata from different issuer/sub identities must never merge.
SELECT gen_random_uuid()::text AS oidc_p1,
       gen_random_uuid()::text AS oidc_p2
\gset
INSERT INTO oidc_identities
    (principal_id, issuer, subject, email, email_verified)
VALUES
    (:'oidc_p1'::uuid, 'https://id-a.example.test', 'subject-a', 'same@example.test', true),
    (:'oidc_p2'::uuid, 'https://id-b.example.test', 'subject-b', 'same@example.test', true);

SELECT count(*) = 2 AS identities_distinct
FROM oidc_identities
WHERE email = 'same@example.test'
  AND principal_id IN (:'oidc_p1'::uuid, :'oidc_p2'::uuid)
\gset
\if :identities_distinct
\else
  \echo 'FAIL: issuer/sub identities were merged by email'
  \quit 1
\endif

-- State consumption is one-use in the database. The application uses the same
-- DELETE ... RETURNING predicate before any token exchange.
INSERT INTO oidc_auth_transactions
    (state, browser_binding, issuer, nonce, pkce_verifier, return_to, expires_at)
VALUES
    (repeat('a', 64), repeat('b', 64), 'https://id.example.test', repeat('c', 64),
     repeat('d', 64), '/', now() + interval '5 minutes');

WITH consumed AS (
    DELETE FROM oidc_auth_transactions
    WHERE state = repeat('a', 64)
      AND browser_binding = repeat('b', 64)
      AND issuer = 'https://id.example.test'
      AND expires_at > now()
    RETURNING 1
)
SELECT count(*) = 1 AS first_consume_wins FROM consumed
\gset
\if :first_consume_wins
\else
  \echo 'FAIL: first OIDC auth-transaction consume did not succeed'
  \quit 1
\endif

WITH replay AS (
    DELETE FROM oidc_auth_transactions
    WHERE state = repeat('a', 64)
    RETURNING 1
)
SELECT count(*) = 0 AS replay_blocked FROM replay
\gset
\if :replay_blocked
\else
  \echo 'FAIL: consumed OIDC state was reusable'
  \quit 1
\endif

-- Current Cedar policy is tenant scoped and visible only with the tenant GUC.
INSERT INTO authorization_policies
    (tenant_id, policy_version, policy_epoch, cedar_source, is_current)
VALUES
    (:'seed_tenant_id'::uuid, '99.99.999', 999999,
     'permit(principal, action, resource);', true);

SELECT count(*) = 1 AS current_policy_visible
FROM authorization_policies
WHERE tenant_id = :'seed_tenant_id'::uuid
  AND is_current
  AND policy_version = '99.99.999'
\gset
\if :current_policy_visible
\else
  \echo 'FAIL: tenant current policy is not visible to runtime authority'
  \quit 1
\endif

-- tower-sessions SQLx store table is writable by the non-owner runtime role.
INSERT INTO tower_sessions.session(id, data, expiry_date)
VALUES ('auth-test-session-rollback', decode('00', 'hex'), now() + interval '1 minute');
SELECT count(*) = 1 AS tower_session_writable
FROM tower_sessions.session
WHERE id = 'auth-test-session-rollback'
\gset
\if :tower_session_writable
\else
  \echo 'FAIL: runtime cannot use tower session storage'
  \quit 1
\endif

RESET ROLE;
ROLLBACK;

\echo 'auth postgres runtime checks passed (rolled back)'
