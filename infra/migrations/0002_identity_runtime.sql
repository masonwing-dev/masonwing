-- MASONWING@1.0.1 WP-005..WP-007 identity/session/current-authority storage.
-- This migration is additive to 0000/0001 and preserves the non-owner
-- `masonwing_app` runtime role plus FORCE RLS on tenant-scoped authority data.

-- Stable membership handles are useful in audit/session views. Permission epoch
-- is distinct from membership_epoch so cached authorization can be invalidated
-- without changing the membership identity.
ALTER TABLE memberships
    ADD COLUMN membership_id uuid NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN permission_epoch bigint GENERATED ALWAYS AS (membership_epoch) STORED;

ALTER TABLE memberships
    ALTER COLUMN permission_epoch DROP EXPRESSION,
    ALTER COLUMN permission_epoch SET DEFAULT 1,
    ALTER COLUMN permission_epoch SET NOT NULL;

ALTER TABLE memberships
    ADD CONSTRAINT memberships_permission_epoch_positive CHECK (permission_epoch > 0),
    ADD CONSTRAINT memberships_membership_id_unique UNIQUE (tenant_id, membership_id);

CREATE OR REPLACE FUNCTION bump_membership_authority_epochs()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.role IS DISTINCT FROM OLD.role OR NEW.status IS DISTINCT FROM OLD.status THEN
        NEW.membership_epoch := GREATEST(NEW.membership_epoch, OLD.membership_epoch + 1);
        NEW.permission_epoch := GREATEST(NEW.permission_epoch, OLD.permission_epoch + 1);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER memberships_bump_authority_epochs
BEFORE UPDATE OF role, status ON memberships
FOR EACH ROW
EXECUTE FUNCTION bump_membership_authority_epochs();

REVOKE ALL ON FUNCTION bump_membership_authority_epochs() FROM PUBLIC;

-- `/session` needs to enumerate the authenticated principal's current active
-- memberships without disabling RLS. The host sets app.principal_id only inside
-- a transaction after resolving the server-side session identity. PostgreSQL
-- permissive SELECT policies OR this principal scope with the existing tenant
-- scope; mutation WITH CHECK remains tenant-scoped by the 0000 policy.
CREATE POLICY membership_principal_self_scope ON memberships
FOR SELECT TO masonwing_app
USING (
    principal_id = NULLIF(current_setting('app.principal_id', true), '')::uuid
);

CREATE POLICY tenant_principal_self_scope ON tenants
FOR SELECT TO masonwing_app
USING (
    EXISTS (
        SELECT 1
        FROM memberships m
        WHERE m.tenant_id = tenants.id
          AND m.principal_id = NULLIF(current_setting('app.principal_id', true), '')::uuid
          AND m.status = 'ACTIVE'
    )
);

-- OIDC identity is issuer + subject. Email is deliberately absent from the
-- uniqueness constraint so equal emails can never auto-link accounts.
CREATE TABLE oidc_identities (
    principal_id uuid PRIMARY KEY,
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    subject text NOT NULL CHECK (length(subject) BETWEEN 1 AND 512),
    email text,
    email_verified boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (issuer, subject)
);

CREATE INDEX oidc_identities_subject_lookup
    ON oidc_identities (issuer, subject);

CREATE OR REPLACE FUNCTION enforce_oidc_identity_immutability()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.principal_id IS DISTINCT FROM OLD.principal_id
       OR NEW.issuer IS DISTINCT FROM OLD.issuer
       OR NEW.subject IS DISTINCT FROM OLD.subject
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'OIDC issuer/subject identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER oidc_identity_key_immutable
BEFORE UPDATE ON oidc_identities
FOR EACH ROW
EXECUTE FUNCTION enforce_oidc_identity_immutability();

REVOKE ALL ON FUNCTION enforce_oidc_identity_immutability() FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE ON oidc_identities TO masonwing_app;

-- Authorization transactions are server-side and consumed with DELETE ...
-- RETURNING, making state/nonce/PKCE one-use even under concurrent callbacks.
-- No OAuth token bytes are persisted here.
CREATE TABLE oidc_auth_transactions (
    state text PRIMARY KEY CHECK (length(state) BETWEEN 32 AND 2048),
    browser_binding text NOT NULL CHECK (length(browser_binding) BETWEEN 32 AND 2048),
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    nonce text NOT NULL CHECK (length(nonce) BETWEEN 16 AND 2048),
    pkce_verifier text NOT NULL CHECK (length(pkce_verifier) BETWEEN 43 AND 256),
    return_to text NOT NULL CHECK (length(return_to) BETWEEN 1 AND 1024),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);

CREATE INDEX oidc_auth_transactions_expiry
    ON oidc_auth_transactions (expires_at);

GRANT SELECT, INSERT, DELETE ON oidc_auth_transactions TO masonwing_app;

-- Immutable Cedar policy versions. `is_current` is only the active pointer;
-- the policy version/source/epoch themselves cannot be rewritten in place.
CREATE TABLE authorization_policies (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    policy_version text NOT NULL
        CHECK (policy_version ~ '^[0-9]+\.[0-9]+\.[0-9]+$'),
    policy_epoch bigint NOT NULL CHECK (policy_epoch > 0),
    cedar_source text NOT NULL CHECK (length(cedar_source) > 0),
    is_current boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, policy_version),
    UNIQUE (tenant_id, policy_epoch)
);

CREATE UNIQUE INDEX authorization_policies_one_current_per_tenant
    ON authorization_policies (tenant_id)
    WHERE is_current;

ALTER TABLE authorization_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE authorization_policies FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_scope ON authorization_policies
USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

CREATE OR REPLACE FUNCTION enforce_policy_version_immutability()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.policy_version IS DISTINCT FROM OLD.policy_version
       OR NEW.policy_epoch IS DISTINCT FROM OLD.policy_epoch
       OR NEW.cedar_source IS DISTINCT FROM OLD.cedar_source
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'authorization policy versions are immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER authorization_policy_versions_immutable
BEFORE UPDATE ON authorization_policies
FOR EACH ROW
EXECUTE FUNCTION enforce_policy_version_immutability();

REVOKE ALL ON FUNCTION enforce_policy_version_immutability() FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE ON authorization_policies TO masonwing_app;

-- Schema/table shape matches tower-sessions-sqlx-store 0.15 PostgresStore.
-- Keeping the DDL in the numbered migration avoids a runtime schema-creation
-- privilege and lets bootstrap/release qualification inspect exact bytes.
CREATE SCHEMA IF NOT EXISTS tower_sessions;
CREATE TABLE IF NOT EXISTS tower_sessions.session (
    id text PRIMARY KEY NOT NULL,
    data bytea NOT NULL,
    expiry_date timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS tower_sessions_session_expiry
    ON tower_sessions.session (expiry_date);

GRANT USAGE ON SCHEMA tower_sessions TO masonwing_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON tower_sessions.session TO masonwing_app;
