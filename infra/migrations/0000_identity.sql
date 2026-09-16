-- Initial typed storage boundary. OIDC sessions belong to the maintained session adapter.
CREATE TABLE tenants (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    status text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'SUSPENDED')),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE memberships (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    principal_id uuid NOT NULL,
    role text NOT NULL CHECK (role IN ('OWNER','EDITOR','REVIEWER','PUBLISHER','VIEWER','SERVICE','SUPPORT')),
    status text NOT NULL CHECK (status IN ('INVITED','ACTIVE','REVOKED')),
    membership_epoch bigint NOT NULL CHECK (membership_epoch > 0),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    PRIMARY KEY (tenant_id, principal_id)
);
ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenants FORCE ROW LEVEL SECURITY;
ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_scope ON tenants USING (id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
CREATE POLICY tenant_scope ON memberships USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
GRANT USAGE ON SCHEMA public TO masonwing_app;
GRANT SELECT ON tenants TO masonwing_app;
GRANT SELECT, INSERT, UPDATE ON memberships TO masonwing_app;
-- Last-owner race and current-authority logic are application acceptance work, not proven by this DDL.
