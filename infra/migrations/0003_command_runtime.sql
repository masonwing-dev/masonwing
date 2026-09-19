-- MASONWING@1.0.1 WP-001/002/003/008/009/010/011/019.
-- Additive runtime storage. The typed tables own state; resource_projections is
-- a versioned read index, never a generic write model for domain plugins.
ALTER TABLE outbox_events ADD COLUMN aggregate_type text NOT NULL DEFAULT 'LegacyRecord';
ALTER TABLE outbox_events ADD COLUMN principal_id uuid;
ALTER TABLE outbox_events ADD COLUMN correlation_id uuid;
ALTER TABLE outbox_events ADD COLUMN delivery_attempts integer NOT NULL DEFAULT 0;
ALTER TABLE outbox_events ADD COLUMN available_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE outbox_events ADD COLUMN lease_expires_at timestamptz;
ALTER TABLE outbox_events ADD COLUMN stream_sequence bigint GENERATED ALWAYS AS IDENTITY;
CREATE INDEX outbox_events_dispatch ON outbox_events(delivery_status,available_at,stream_sequence);
GRANT USAGE,SELECT ON SEQUENCE outbox_events_stream_sequence_seq TO masonwing_app;
CREATE TABLE command_receipts (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    principal_id uuid NOT NULL,
    operation text NOT NULL CHECK (length(operation) BETWEEN 1 AND 128),
    idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    fingerprint text NOT NULL CHECK (fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    command_id uuid NOT NULL,
    receipt jsonb NOT NULL CHECK (jsonb_typeof(receipt) = 'object'),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, principal_id, operation, idempotency_key)
);
CREATE INDEX command_receipts_expiry ON command_receipts(expires_at);

CREATE TABLE artifacts (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    digest text NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    schema_version text NOT NULL DEFAULT '1.0.0',
    classification text NOT NULL CHECK (classification IN ('PUBLIC','INTERNAL','CONFIDENTIAL','RESTRICTED')),
    content_type text NOT NULL CHECK (length(content_type) BETWEEN 1 AND 128),
    size_bytes bigint NOT NULL CHECK (size_bytes BETWEEN 1 AND 1073741824),
    storage_key text NOT NULL,
    state text NOT NULL CHECK (state IN ('PENDING','ACTIVE','QUARANTINED','TOMBSTONED','PURGED')),
    creator_id uuid NOT NULL,
    legal_hold boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (storage_key)
);
CREATE INDEX artifacts_digest_scope ON artifacts(tenant_id, digest) WHERE state='ACTIVE';
CREATE TABLE uploads (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    principal_id uuid NOT NULL,
    expires_at timestamptz NOT NULL,
    state text NOT NULL CHECK (state IN ('OPEN','UPLOADED','FINALIZED','EXPIRED','FAILED')),
    observed_digest text,
    observed_size_bytes bigint,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, artifact_id),
    FOREIGN KEY (tenant_id, artifact_id) REFERENCES artifacts(tenant_id, id)
);
CREATE TABLE deletion_tombstones (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    requested_by uuid NOT NULL,
    reason text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, resource_type, resource_id)
);

CREATE TABLE resource_projections (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    version integer NOT NULL CHECK (version > 0),
    artifact_id uuid NOT NULL,
    search_label text NOT NULL DEFAULT '' CHECK (length(search_label) <= 512),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    revision_sequence bigint GENERATED ALWAYS AS IDENTITY,
    PRIMARY KEY (tenant_id, resource_type, resource_id, version),
    FOREIGN KEY (tenant_id, artifact_id) REFERENCES artifacts(tenant_id, id)
);
CREATE INDEX resource_projections_page ON resource_projections(tenant_id,resource_type,created_at,resource_id,version DESC);
CREATE TABLE read_cursors (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    principal_id uuid NOT NULL,
    membership_epoch bigint NOT NULL,
    permission_epoch bigint NOT NULL,
    policy_version text NOT NULL,
    policy_epoch bigint NOT NULL,
    resource_type text NOT NULL,
    query_digest text NOT NULL,
    snapshot_at timestamptz NOT NULL,
    snapshot_sequence bigint NOT NULL,
    last_created_at timestamptz NOT NULL,
    last_resource_id text NOT NULL,
    page_limit integer NOT NULL CHECK (page_limit BETWEEN 1 AND 200),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, id)
);

CREATE TABLE publisher_keys (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    publisher_id text NOT NULL,
    key_id text NOT NULL,
    public_key bytea NOT NULL CHECK (octet_length(public_key)=32),
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, publisher_id, key_id)
);
CREATE TABLE artifact_signatures (
    tenant_id uuid NOT NULL,
    id text NOT NULL,
    publisher_id text NOT NULL,
    key_id text NOT NULL,
    artifact_digest text NOT NULL CHECK (artifact_digest ~ '^sha256:[0-9a-f]{64}$'),
    manifest_digest text NOT NULL CHECK (manifest_digest ~ '^sha256:[0-9a-f]{64}$'),
    signature bytea NOT NULL CHECK (octet_length(signature)=64),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    FOREIGN KEY (tenant_id,publisher_id,key_id) REFERENCES publisher_keys(tenant_id,publisher_id,key_id)
);
CREATE TABLE plugin_versions (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    plugin_id text NOT NULL,
    artifact_digest text NOT NULL CHECK (artifact_digest ~ '^sha256:[0-9a-f]{64}$'),
    manifest jsonb NOT NULL CHECK (jsonb_typeof(manifest)='object'),
    contract_version text NOT NULL,
    plugin_version text NOT NULL,
    signature_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,plugin_id,artifact_digest),
    UNIQUE (tenant_id,plugin_id,plugin_version),
    FOREIGN KEY (tenant_id,signature_id) REFERENCES artifact_signatures(tenant_id,id)
);
CREATE TABLE plugin_installations (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    plugin_id text NOT NULL,
    artifact_digest text NOT NULL,
    state text NOT NULL CHECK (state IN ('INSTALLED_DISABLED','ENABLED','DRAINING','DISABLED','REVOKED','UNINSTALLED')),
    granted_capabilities text[] NOT NULL DEFAULT '{}',
    revocation_epoch bigint NOT NULL DEFAULT 1 CHECK (revocation_epoch > 0),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,plugin_id),
    FOREIGN KEY (tenant_id,plugin_id,artifact_digest) REFERENCES plugin_versions(tenant_id,plugin_id,artifact_digest)
);
CREATE TABLE plugin_revocations (
    tenant_id uuid NOT NULL,
    plugin_id text NOT NULL,
    artifact_digest text NOT NULL,
    reason text NOT NULL,
    revoked_by uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,plugin_id,artifact_digest),
    FOREIGN KEY (tenant_id,plugin_id,artifact_digest) REFERENCES plugin_versions(tenant_id,plugin_id,artifact_digest)
);
CREATE TABLE product_compositions (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    product_id text NOT NULL,
    lock_artifact_id uuid NOT NULL,
    lock_digest text NOT NULL,
    enabled_plugins text[] NOT NULL,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,product_id),
    FOREIGN KEY (tenant_id,lock_artifact_id) REFERENCES artifacts(tenant_id,id)
);
CREATE TABLE delegations (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    principal_id text NOT NULL,
    principal_type text NOT NULL CHECK (principal_type IN ('USER','SERVICE','AUTOMATION','SUPPORT')),
    issuer text NOT NULL,
    creator_id uuid NOT NULL,
    actions text[] NOT NULL,
    resources jsonb NOT NULL CHECK (jsonb_typeof(resources)='array'),
    parent_grant_id uuid,
    expires_at timestamptz NOT NULL,
    state text NOT NULL CHECK (state IN ('ACTIVE','REVOKED','EXPIRED')),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    fence bigint NOT NULL DEFAULT 1 CHECK (fence > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    FOREIGN KEY (tenant_id,parent_grant_id) REFERENCES delegations(tenant_id,id)
);
CREATE TABLE workflow_definitions (
    tenant_id uuid NOT NULL,
    workflow_id text NOT NULL,
    workflow_version text NOT NULL,
    plugin_id text NOT NULL,
    plugin_digest text NOT NULL,
    definition_digest text NOT NULL,
    definition jsonb NOT NULL CHECK (jsonb_typeof(definition)='object'),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,workflow_id,workflow_version,plugin_digest),
    FOREIGN KEY (tenant_id,plugin_id,plugin_digest) REFERENCES plugin_versions(tenant_id,plugin_id,artifact_digest)
);
CREATE TABLE runs (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    principal_id uuid NOT NULL,
    workflow_id text NOT NULL,
    workflow_version text NOT NULL,
    plugin_id text NOT NULL,
    plugin_digest text NOT NULL,
    grant_id uuid NOT NULL,
    grant_fence bigint NOT NULL,
    input_ref jsonb NOT NULL CHECK (jsonb_typeof(input_ref)='object'),
    state text NOT NULL CHECK (state IN ('QUEUED','RUNNING','WAITING_APPROVAL','WAITING_RETRY','BLOCKED','CANCEL_REQUESTED','SUCCEEDED','FAILED','CANCELLED')),
    fence bigint NOT NULL DEFAULT 1 CHECK (fence > 0),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    temporal_workflow_id text NOT NULL,
    temporal_run_id text,
    dispatch_state text NOT NULL DEFAULT 'PENDING' CHECK (dispatch_state IN ('PENDING','STARTED','BLOCKED')),
    result_ref jsonb,
    failure_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (temporal_workflow_id),
    FOREIGN KEY (tenant_id,grant_id) REFERENCES delegations(tenant_id,id),
    FOREIGN KEY (tenant_id,plugin_id,plugin_digest) REFERENCES plugin_versions(tenant_id,plugin_id,artifact_digest)
);
CREATE INDEX runs_admission ON runs(tenant_id,state,created_at);
CREATE TABLE run_checkpoints (
    tenant_id uuid NOT NULL,
    run_id uuid NOT NULL,
    logical_step_id text NOT NULL,
    fence bigint NOT NULL CHECK (fence > 0),
    state text NOT NULL CHECK (state IN ('RUNNING','SUCCEEDED','FAILED','BLOCKED','INCOMPLETE')),
    output_ref jsonb,
    failure_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,run_id,logical_step_id),
    FOREIGN KEY (tenant_id,run_id) REFERENCES runs(tenant_id,id)
);

DO $$
DECLARE relation_name text;
BEGIN
    FOREACH relation_name IN ARRAY ARRAY['command_receipts','artifacts','uploads','deletion_tombstones',
        'resource_projections','read_cursors','publisher_keys','artifact_signatures','plugin_versions',
        'plugin_installations','plugin_revocations','product_compositions','delegations','workflow_definitions',
        'runs','run_checkpoints']
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('CREATE POLICY tenant_scope ON %I USING (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid) WITH CHECK (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid)',relation_name);
    END LOOP;
END $$;
GRANT SELECT,INSERT,UPDATE ON command_receipts,artifacts,uploads,plugin_installations,product_compositions,delegations,runs,run_checkpoints TO masonwing_app;
GRANT SELECT,INSERT ON deletion_tombstones,resource_projections,read_cursors,plugin_versions,plugin_revocations,workflow_definitions TO masonwing_app;
-- Trust material is provisioned by the operator CLI, never arbitrary tenant commands.
GRANT SELECT ON publisher_keys,artifact_signatures TO masonwing_app;
GRANT USAGE,SELECT ON SEQUENCE resource_projections_revision_sequence_seq TO masonwing_app;
