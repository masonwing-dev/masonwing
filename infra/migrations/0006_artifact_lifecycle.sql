-- MASONWING@1.0.1 WP-010: upload attempts have a lease separate from visibility.
-- Numbered SQLx migration: after application, preserve these exact bytes.
ALTER TABLE uploads ADD COLUMN lease_id uuid;
ALTER TABLE uploads ADD COLUMN lease_expires_at timestamptz;
ALTER TABLE uploads ADD COLUMN scan_code text;
ALTER TABLE uploads ADD COLUMN scanned_at timestamptz;
CREATE INDEX uploads_open_leases ON uploads(tenant_id,lease_expires_at) WHERE state='OPEN';

CREATE TABLE artifact_lineage (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    source_id uuid NOT NULL,
    derived_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,source_id,derived_id),
    FOREIGN KEY (tenant_id,source_id) REFERENCES artifacts(tenant_id,id),
    FOREIGN KEY (tenant_id,derived_id) REFERENCES artifacts(tenant_id,id),
    CHECK (source_id<>derived_id)
);
CREATE TABLE artifact_rights (
    tenant_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    rights_id uuid NOT NULL,
    access_mode text NOT NULL CHECK (access_mode IN ('FIRST_PARTY','AUTHORIZED_API','LICENSED','PERMITTED_CRAWL','MANUAL_IMPORT')),
    allow_acquire boolean NOT NULL,
    allow_ai_analysis boolean NOT NULL,
    allow_transform boolean NOT NULL,
    allow_public_redistribution boolean NOT NULL,
    allowed_provider_profiles text[] NOT NULL DEFAULT '{}',
    expires_at timestamptz,
    evidence_ref jsonb,
    state text NOT NULL CHECK (state IN ('ACTIVE','REVOKED','EXPIRED')),
    version integer NOT NULL DEFAULT 1 CHECK (version>0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,artifact_id),
    UNIQUE (tenant_id,rights_id),
    FOREIGN KEY (tenant_id,artifact_id) REFERENCES artifacts(tenant_id,id)
);
CREATE TABLE deletion_requests (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    state text NOT NULL CHECK (state IN ('TOMBSTONED','PENDING_EXTERNAL','PURGE_PENDING','PURGED')),
    affected_artifact_ids uuid[] NOT NULL,
    requested_by uuid NOT NULL,
    reason text NOT NULL,
    version integer NOT NULL DEFAULT 1 CHECK (version>0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id)
);
DO $$
DECLARE relation_name text;
BEGIN
    FOREACH relation_name IN ARRAY ARRAY['artifact_lineage','artifact_rights','deletion_requests'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('CREATE POLICY tenant_scope ON %I USING (tenant_id = NULLIF(current_setting(''app.tenant_id'',true),'''')::uuid) WITH CHECK (tenant_id = NULLIF(current_setting(''app.tenant_id'',true),'''')::uuid)',relation_name);
    END LOOP;
END $$;
GRANT SELECT,INSERT ON artifact_lineage TO masonwing_app;
GRANT SELECT,INSERT,UPDATE ON artifact_rights,deletion_requests TO masonwing_app;
