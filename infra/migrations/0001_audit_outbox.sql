CREATE TABLE audit_events (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    principal_id uuid NOT NULL,
    action text NOT NULL,
    resource_type text NOT NULL,
    resource_id uuid NOT NULL,
    correlation_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id)
);
CREATE TABLE outbox_events (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    aggregate_id uuid NOT NULL,
    aggregate_version integer NOT NULL CHECK (aggregate_version > 0),
    event_name text NOT NULL,
    artifact_refs jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(artifact_refs) = 'array'),
    delivery_status text NOT NULL DEFAULT 'PENDING' CHECK (delivery_status IN ('PENDING','DELIVERED','DEAD_LETTER')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, aggregate_id, aggregate_version, event_name)
);
CREATE TABLE inbox_events (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    consumer text NOT NULL,
    event_id uuid NOT NULL,
    aggregate_sequence bigint NOT NULL CHECK (aggregate_sequence >= 0),
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, consumer, event_id)
);
ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_events FORCE ROW LEVEL SECURITY;
ALTER TABLE outbox_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE outbox_events FORCE ROW LEVEL SECURITY;
ALTER TABLE inbox_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE inbox_events FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_scope ON audit_events USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
CREATE POLICY tenant_scope ON outbox_events USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
CREATE POLICY tenant_scope ON inbox_events USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
GRANT SELECT, INSERT ON audit_events, inbox_events TO masonwing_app;
GRANT SELECT, INSERT, UPDATE ON outbox_events TO masonwing_app;
