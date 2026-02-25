-- ============================================================
-- 002: Tenants (Multi-tenancy isolation layer)
-- ============================================================
-- Tenant — isolated runtime environment within an organization.
-- One org can have multiple tenants (e.g., prod/staging/dev).
-- ============================================================

CREATE TYPE tenant_status AS ENUM ('active', 'suspended', 'archived');

CREATE TABLE tenants (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    slug            VARCHAR(64) NOT NULL,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    status          tenant_status NOT NULL DEFAULT 'active',
    is_default      BOOLEAN NOT NULL DEFAULT FALSE,

    -- Quotas & limits
    max_concurrent_executions   INTEGER NOT NULL DEFAULT 10,
    max_workflows               INTEGER NOT NULL DEFAULT 100,
    max_credentials             INTEGER NOT NULL DEFAULT 50,
    max_memory_mb               INTEGER NOT NULL DEFAULT 512,
    execution_timeout_secs      INTEGER NOT NULL DEFAULT 300,

    -- Runtime config (overrides engine defaults)
    config          JSONB NOT NULL DEFAULT '{}',

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (organization_id, slug)
);

CREATE INDEX idx_tenants_org ON tenants(organization_id);
CREATE INDEX idx_tenants_status ON tenants(status);

-- Ensure only one default tenant per org
CREATE UNIQUE INDEX idx_tenants_default_per_org
    ON tenants(organization_id)
    WHERE is_default = TRUE;

-- ============================================================
-- TENANT SETTINGS (key-value for extensibility)
-- ============================================================

CREATE TABLE tenant_settings (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id   UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key         VARCHAR(255) NOT NULL,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, key)
);

CREATE INDEX idx_tenant_settings_tenant ON tenant_settings(tenant_id);

-- ============================================================
-- TENANT VARIABLE STORE (shared env vars for workflows)
-- ============================================================

CREATE TABLE tenant_variables (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key             VARCHAR(255) NOT NULL,
    value           TEXT NOT NULL,                   -- stored as text, can be encrypted
    is_secret       BOOLEAN NOT NULL DEFAULT FALSE,  -- masked in UI if true
    description     TEXT,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, key)
);

CREATE INDEX idx_tenant_vars_tenant ON tenant_variables(tenant_id);
