-- 006: Tenant Configuration (settings + variables)

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
