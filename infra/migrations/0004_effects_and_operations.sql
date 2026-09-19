-- MASONWING@1.0.1 WP-012/013/014/015/016/017/018/019/020/021/022.
CREATE TABLE budget_settings (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    period text NOT NULL CHECK (period IN ('RUN','DAY','MONTH')),
    limit_microunits bigint NOT NULL CHECK (limit_microunits BETWEEN 0 AND 9007199254740991),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,currency,period)
);
CREATE TABLE budget_accounts (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    account_key text NOT NULL,
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    limit_microunits bigint NOT NULL CHECK (limit_microunits BETWEEN 0 AND 9007199254740991),
    held_microunits bigint NOT NULL DEFAULT 0 CHECK (held_microunits >= 0),
    charged_microunits bigint NOT NULL DEFAULT 0 CHECK (charged_microunits >= 0),
    PRIMARY KEY (tenant_id,account_key),
    CHECK (held_microunits <= limit_microunits - charged_microunits)
);
CREATE TABLE cost_reservations (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    run_id uuid NOT NULL,
    logical_call_id text NOT NULL,
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    upper_bound_microunits bigint NOT NULL CHECK (upper_bound_microunits BETWEEN 0 AND 9007199254740991),
    settled_microunits bigint CHECK (settled_microunits BETWEEN 0 AND upper_bound_microunits),
    state text NOT NULL CHECK (state IN ('RESERVED','SETTLED','UNKNOWN','RELEASED')),
    price_profile text NOT NULL,
    account_keys text[] NOT NULL,
    usage_fingerprint text,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,run_id,logical_call_id),
    FOREIGN KEY (tenant_id,run_id) REFERENCES runs(tenant_id,id),
    CHECK ((state='SETTLED') = (settled_microunits IS NOT NULL))
);
CREATE TABLE cost_ledger (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    reservation_id uuid NOT NULL,
    event_type text NOT NULL CHECK (event_type IN ('RESERVED','UNKNOWN','SETTLED','RELEASED')),
    held_delta bigint NOT NULL,
    charged_delta bigint NOT NULL,
    usage_fingerprint text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,reservation_id,event_type),
    FOREIGN KEY (tenant_id,reservation_id) REFERENCES cost_reservations(tenant_id,id)
);
CREATE TABLE connections (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    provider text NOT NULL,
    external_account text NOT NULL,
    scopes text[] NOT NULL,
    secret_ref text NOT NULL,
    state text NOT NULL CHECK (state IN ('PENDING','ACTIVE','NEEDS_REAUTH','PAUSED','REVOKED')),
    contract_version text NOT NULL,
    capabilities text[] NOT NULL,
    verified_at timestamptz,
    credential_version bigint NOT NULL DEFAULT 1,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id)
);
CREATE TABLE connector_authorizations (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    principal_id uuid NOT NULL,
    connection_id uuid NOT NULL,
    state_digest text NOT NULL UNIQUE,
    authorization_url text NOT NULL,
    return_to text NOT NULL,
    secret_ref text NOT NULL,
    requested_scopes text[] NOT NULL,
    state text NOT NULL CHECK (state IN ('PENDING','COMPLETED','EXPIRED','FAILED')),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    FOREIGN KEY (tenant_id,connection_id) REFERENCES connections(tenant_id,id)
);
CREATE TABLE approvals (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    run_id uuid NOT NULL,
    action text NOT NULL,
    target jsonb NOT NULL CHECK (jsonb_typeof(target)='object'),
    content_digest text NOT NULL CHECK (content_digest ~ '^sha256:[0-9a-f]{64}$'),
    scope_digest text NOT NULL CHECK (scope_digest ~ '^sha256:[0-9a-f]{64}$'),
    policy_version text NOT NULL,
    policy_epoch bigint NOT NULL,
    max_cost_microunits bigint NOT NULL CHECK (max_cost_microunits BETWEEN 0 AND 9007199254740991),
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    expires_at timestamptz NOT NULL,
    created_by uuid NOT NULL,
    decided_by uuid,
    decided_issuer text,
    state text NOT NULL CHECK (state IN ('REQUESTED','APPROVED','REJECTED','CANCELLED','EXPIRED','INVALIDATED')),
    decision_reason text,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    FOREIGN KEY (tenant_id,run_id) REFERENCES runs(tenant_id,id)
);
CREATE TABLE effects (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    run_id uuid NOT NULL,
    grant_id uuid NOT NULL,
    connection_id uuid NOT NULL,
    action text NOT NULL,
    target jsonb NOT NULL CHECK (jsonb_typeof(target)='object'),
    content_ref jsonb NOT NULL CHECK (jsonb_typeof(content_ref)='object'),
    fingerprint text NOT NULL CHECK (fingerprint ~ '^sha256:[0-9a-f]{64}$'),
    idempotency_key text NOT NULL,
    approval_id uuid,
    reservation_id uuid NOT NULL,
    state text NOT NULL CHECK (state IN ('PREPARED','WAITING_APPROVAL','AUTHORIZED','EXECUTING','SUCCEEDED','FAILED_CONFIRMED','OUTCOME_UNKNOWN','RECONCILING','MANUAL_REVIEW','CANCELLED')),
    transmit_count integer NOT NULL DEFAULT 0 CHECK (transmit_count >= 0),
    dispatch_fence bigint NOT NULL DEFAULT 1 CHECK (dispatch_fence > 0),
    lease_expires_at timestamptz,
    reconciliation_queued boolean NOT NULL DEFAULT false,
    original_effect_id uuid,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,connection_id,idempotency_key),
    FOREIGN KEY (tenant_id,run_id) REFERENCES runs(tenant_id,id),
    FOREIGN KEY (tenant_id,grant_id) REFERENCES delegations(tenant_id,id),
    FOREIGN KEY (tenant_id,connection_id) REFERENCES connections(tenant_id,id),
    FOREIGN KEY (tenant_id,approval_id) REFERENCES approvals(tenant_id,id),
    FOREIGN KEY (tenant_id,reservation_id) REFERENCES cost_reservations(tenant_id,id),
    FOREIGN KEY (tenant_id,original_effect_id) REFERENCES effects(tenant_id,id)
);
CREATE TABLE effect_receipts (
    tenant_id uuid NOT NULL,
    id uuid NOT NULL,
    effect_id uuid NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('SUCCEEDED','FAILED_CONFIRMED','PROVEN_ABSENT')),
    remote_id text,
    remote_version text,
    remote_url text,
    evidence_ref jsonb NOT NULL CHECK (jsonb_typeof(evidence_ref)='object'),
    provider_contract_version text NOT NULL,
    observed_at timestamptz NOT NULL,
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,effect_id),
    FOREIGN KEY (tenant_id,effect_id) REFERENCES effects(tenant_id,id)
);
CREATE TABLE kill_switches (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    scope text NOT NULL CHECK (scope IN ('TENANT','PLUGIN','CONNECTION')),
    target_id text NOT NULL,
    active boolean NOT NULL,
    reason text NOT NULL,
    changed_by uuid NOT NULL,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,scope,target_id)
);
CREATE TABLE provider_profiles (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    profile_id text NOT NULL,
    profile jsonb NOT NULL CHECK (jsonb_typeof(profile)='object'),
    credential_ref text NOT NULL,
    qualification_ref jsonb,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,profile_id)
);
CREATE TABLE notifications (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    recipient_id uuid NOT NULL,
    event_id uuid NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    subject_code text NOT NULL,
    read_at timestamptz,
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,recipient_id,event_id)
);
CREATE TABLE ui_contributions (
    tenant_id uuid NOT NULL,
    plugin_id text NOT NULL,
    contribution_id text NOT NULL,
    contribution jsonb NOT NULL CHECK (jsonb_typeof(contribution)='object'),
    path text NOT NULL,
    slot text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,plugin_id,contribution_id),
    UNIQUE (tenant_id,path,slot)
);
CREATE TABLE support_grants (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    requested_by uuid NOT NULL,
    actions text[] NOT NULL,
    reason text NOT NULL,
    expires_at timestamptz NOT NULL,
    state text NOT NULL CHECK (state IN ('APPROVED','REVOKED','EXPIRED')),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id)
);
CREATE TABLE catalog_listings (
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    id uuid NOT NULL,
    publisher_id text NOT NULL,
    plugin_id text NOT NULL,
    artifact_digest text NOT NULL,
    manifest jsonb NOT NULL,
    support_contact text NOT NULL,
    description text NOT NULL,
    submitted_by uuid NOT NULL,
    reviewed_by uuid,
    evidence_ref jsonb,
    state text NOT NULL CHECK (state IN ('PENDING','APPROVED','REJECTED','REVOKED')),
    version integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id,id),
    UNIQUE (tenant_id,plugin_id,artifact_digest)
);

DO $$
DECLARE relation_name text;
BEGIN
    FOREACH relation_name IN ARRAY ARRAY['budget_settings','budget_accounts','cost_reservations','cost_ledger',
        'connections','connector_authorizations','approvals','effects','effect_receipts','kill_switches',
        'provider_profiles','notifications','ui_contributions','support_grants','catalog_listings']
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY',relation_name);
        EXECUTE format('CREATE POLICY tenant_scope ON %I USING (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid) WITH CHECK (tenant_id = NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid)',relation_name);
    END LOOP;
END $$;
GRANT SELECT,INSERT,UPDATE ON budget_settings,budget_accounts,cost_reservations,connections,connector_authorizations,approvals,effects,kill_switches,provider_profiles,notifications,support_grants,catalog_listings TO masonwing_app;
GRANT SELECT,INSERT ON cost_ledger,effect_receipts,ui_contributions TO masonwing_app;
