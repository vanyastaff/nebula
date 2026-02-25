-- ============================================================
-- 009: Workflows
-- ============================================================

CREATE TYPE workflow_status AS ENUM ('draft', 'active', 'inactive', 'archived');
CREATE TYPE trigger_type AS ENUM ('manual', 'webhook', 'schedule', 'event', 'form');

CREATE TABLE workflows (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    status          workflow_status NOT NULL DEFAULT 'draft',

    -- Active version reference (denormalized for fast lookup)
    active_version_id UUID,                                -- FK added after workflow_versions

    -- Stats (updated by triggers/background job)
    total_executions    BIGINT NOT NULL DEFAULT 0,
    last_executed_at    TIMESTAMPTZ,
    avg_duration_ms     INTEGER,

    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_workflows_tenant ON workflows(tenant_id);
CREATE INDEX idx_workflows_status ON workflows(status);
