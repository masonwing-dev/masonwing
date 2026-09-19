-- MASONWING@1.0.1 WP-005..WP-007 identity command persistence.
-- Numbered SQLx migration: after application, preserve these exact bytes.

CREATE TABLE membership_roles (
    tenant_id uuid NOT NULL,
    membership_id uuid NOT NULL,
    role text NOT NULL CHECK (role IN ('OWNER','EDITOR','REVIEWER','PUBLISHER','VIEWER','SERVICE','SUPPORT')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, membership_id, role),
    FOREIGN KEY (tenant_id, membership_id)
        REFERENCES memberships(tenant_id, membership_id) ON DELETE CASCADE
);

-- memberships is FORCE RLS, including for its owner. Grant the fixed migration
-- role a temporary, explicit SELECT policy for this one backfill, then remove
-- it before runtime grants become relevant.
CREATE POLICY membership_roles_migration_backfill ON memberships
FOR SELECT TO masonwing_migrator USING (true);

INSERT INTO membership_roles (tenant_id, membership_id, role)
SELECT tenant_id, membership_id, role
FROM memberships;

DROP POLICY membership_roles_migration_backfill ON memberships;

ALTER TABLE membership_roles ENABLE ROW LEVEL SECURITY;
ALTER TABLE membership_roles FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_scope ON membership_roles
USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
GRANT SELECT, INSERT, DELETE ON membership_roles TO masonwing_app;

-- Enumerating a verified principal's memberships is a separate read mode. Once
-- a transaction selects a tenant, only tenant_scope can authorize its rows.
ALTER POLICY membership_principal_self_scope ON memberships
USING (
    NULLIF(current_setting('app.tenant_id',true),'') IS NULL
    AND principal_id=NULLIF(current_setting('app.principal_id',true),'')::uuid
);
ALTER POLICY tenant_principal_self_scope ON tenants
USING (
    NULLIF(current_setting('app.tenant_id',true),'') IS NULL
    AND EXISTS (
        SELECT 1 FROM memberships m
        WHERE m.tenant_id=tenants.id
          AND m.principal_id=NULLIF(current_setting('app.principal_id',true),'')::uuid
          AND m.status='ACTIVE'
    )
);
CREATE POLICY membership_role_principal_self_scope ON membership_roles
FOR SELECT TO masonwing_app
USING (
    NULLIF(current_setting('app.tenant_id',true),'') IS NULL
    AND EXISTS (
        SELECT 1 FROM memberships m
        WHERE m.tenant_id=membership_roles.tenant_id
          AND m.membership_id=membership_roles.membership_id
          AND m.principal_id=NULLIF(current_setting('app.principal_id',true),'')::uuid
          AND m.status='ACTIVE'
    )
);

CREATE TABLE membership_invites (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    invite_id uuid NOT NULL,
    email_normalized text NOT NULL CHECK (length(email_normalized) BETWEEN 3 AND 320),
    roles text[] NOT NULL CHECK (
        cardinality(roles) BETWEEN 1 AND 10
        AND roles <@ ARRAY['OWNER','EDITOR','REVIEWER','PUBLISHER','VIEWER','SERVICE','SUPPORT']::text[]
    ),
    expires_at timestamptz NOT NULL,
    token_digest text NOT NULL UNIQUE CHECK (token_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_by uuid NOT NULL,
    secret_retrieved_at timestamptz,
    consumed_at timestamptz,
    consumed_by uuid,
    membership_id uuid,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, invite_id),
    FOREIGN KEY (tenant_id, membership_id)
        REFERENCES memberships(tenant_id, membership_id),
    CHECK (expires_at > created_at),
    CHECK (
        (consumed_at IS NULL AND consumed_by IS NULL AND membership_id IS NULL)
        OR (consumed_at IS NOT NULL AND consumed_by IS NOT NULL AND membership_id IS NOT NULL)
    )
);
CREATE INDEX membership_invites_expiry ON membership_invites (tenant_id, expires_at)
WHERE consumed_at IS NULL;

CREATE OR REPLACE FUNCTION enforce_membership_invite_integrity()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.invite_id IS DISTINCT FROM OLD.invite_id
       OR NEW.email_normalized IS DISTINCT FROM OLD.email_normalized
       OR NEW.roles IS DISTINCT FROM OLD.roles
       OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
       OR NEW.token_digest IS DISTINCT FROM OLD.token_digest
       OR NEW.created_by IS DISTINCT FROM OLD.created_by
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'membership invitation authority fields are immutable';
    END IF;
    IF OLD.secret_retrieved_at IS NOT NULL
       AND NEW.secret_retrieved_at IS DISTINCT FROM OLD.secret_retrieved_at THEN
        RAISE EXCEPTION 'membership invitation secret retrieval is one-use';
    END IF;
    IF OLD.consumed_at IS NOT NULL
       AND (NEW.consumed_at IS DISTINCT FROM OLD.consumed_at
            OR NEW.consumed_by IS DISTINCT FROM OLD.consumed_by
            OR NEW.membership_id IS DISTINCT FROM OLD.membership_id) THEN
        RAISE EXCEPTION 'membership invitation consumption is immutable';
    END IF;
    IF NEW.version < OLD.version OR NEW.version > OLD.version + 1 THEN
        RAISE EXCEPTION 'membership invitation version transition is invalid';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER membership_invites_integrity
BEFORE UPDATE ON membership_invites
FOR EACH ROW
EXECUTE FUNCTION enforce_membership_invite_integrity();

REVOKE ALL ON FUNCTION enforce_membership_invite_integrity() FROM PUBLIC;

ALTER TABLE membership_invites ENABLE ROW LEVEL SECURITY;
ALTER TABLE membership_invites FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_scope ON membership_invites
USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
GRANT SELECT, INSERT, UPDATE ON membership_invites TO masonwing_app;

CREATE TABLE identity_configuration_proposals (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    client_id text NOT NULL CHECK (length(client_id) BETWEEN 1 AND 128),
    client_secret_ref text NOT NULL CHECK (length(client_secret_ref) BETWEEN 1 AND 128),
    allowed_redirect_uris text[] NOT NULL CHECK (cardinality(allowed_redirect_uris) <= 10),
    state text NOT NULL DEFAULT 'PENDING' CHECK (state IN ('PENDING','APPROVED','REJECTED','SUPERSEDED')),
    runtime_state text NOT NULL DEFAULT 'UNQUALIFIED' CHECK (runtime_state IN ('UNQUALIFIED','QUALIFIED')),
    created_by uuid NOT NULL,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id)
);

CREATE TABLE policy_configuration_state (
    tenant_id uuid PRIMARY KEY REFERENCES tenants(id),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE policy_proposals (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    policy_ref jsonb NOT NULL CHECK (jsonb_typeof(policy_ref)='object'),
    review_ref jsonb NOT NULL CHECK (jsonb_typeof(review_ref)='object'),
    policy_digest text NOT NULL CHECK (policy_digest ~ '^sha256:[0-9a-f]{64}$'),
    cedar_source text NOT NULL CHECK (length(cedar_source) BETWEEN 1 AND 1048576),
    base_policy_version text NOT NULL CHECK (base_policy_version ~ '^[0-9]+\.[0-9]+\.[0-9]+$'),
    base_policy_epoch bigint NOT NULL CHECK (base_policy_epoch > 0),
    state text NOT NULL DEFAULT 'PENDING' CHECK (state IN ('PENDING','APPROVED','REJECTED','SUPERSEDED')),
    created_by uuid NOT NULL,
    version integer NOT NULL CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id)
);

DO $$
DECLARE relation_name text;
BEGIN
    FOREACH relation_name IN ARRAY ARRAY[
        'identity_configuration_proposals',
        'policy_configuration_state',
        'policy_proposals'
    ]
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', relation_name);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', relation_name);
        EXECUTE format(
            'CREATE POLICY tenant_scope ON %I USING (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid) WITH CHECK (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid)',
            relation_name
        );
    END LOOP;
END $$;

GRANT SELECT, INSERT ON identity_configuration_proposals TO masonwing_app;
GRANT SELECT, INSERT, UPDATE ON policy_configuration_state TO masonwing_app;
GRANT SELECT, INSERT ON policy_proposals TO masonwing_app;

-- Published policy versions stay owned by authorization_policies. 0005 creates
-- only pending proposal storage; there is deliberately no trigger or function
-- that can flip authorization_policies.is_current from a proposal.
