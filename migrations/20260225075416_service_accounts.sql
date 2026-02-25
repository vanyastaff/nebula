-- 021: Service Accounts (machine identities)
-- ============================================================
-- Service accounts are non-human principals for CI/CD, SDK integrations,
-- worker agents, etc. They authenticate via API keys or short-lived tokens.
-- ============================================================

CREATE TABLE service_accounts (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    tenant_id       UUID REFERENCES tenants(id) ON DELETE CASCADE,     -- NULL = org-level
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, name)
);

CREATE INDEX idx_sa_org ON service_accounts(organization_id);
CREATE INDEX idx_sa_tenant ON service_accounts(tenant_id) WHERE tenant_id IS NOT NULL;

-- Service account API keys (separate from user api_keys for clear auditing)
CREATE TABLE service_account_keys (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    service_account_id  UUID NOT NULL REFERENCES service_accounts(id) ON DELETE CASCADE,
    name                VARCHAR(255) NOT NULL,
    key_hash            TEXT NOT NULL UNIQUE,
    key_prefix          VARCHAR(16) NOT NULL,               -- 'nsa_xxxx' for display
    scopes              TEXT[] NOT NULL DEFAULT '{}',       -- constrained scopes for this key
    last_used_at        TIMESTAMPTZ,
    expires_at          TIMESTAMPTZ,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    created_by          UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sa_keys_sa ON service_account_keys(service_account_id);
CREATE INDEX idx_sa_keys_hash ON service_account_keys(key_hash);

-- Project membership for service accounts (reuses project_members via principal polymorphism)
-- OR: separate table for clarity (chosen here)
CREATE TABLE service_account_project_roles (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    service_account_id  UUID NOT NULL REFERENCES service_accounts(id) ON DELETE CASCADE,
    project_id          UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    builtin_role        VARCHAR(64) NOT NULL DEFAULT 'project_runner',
    custom_role_id      UUID REFERENCES roles(id) ON DELETE SET NULL,
    granted_by          UUID REFERENCES users(id) ON DELETE SET NULL,
    granted_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (service_account_id, project_id)
);

CREATE INDEX idx_sa_project_roles_sa ON service_account_project_roles(service_account_id);
CREATE INDEX idx_sa_project_roles_project ON service_account_project_roles(project_id);
