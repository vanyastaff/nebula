-- ============================================================
-- 005: Workflows (core definitions + versioning)
-- ============================================================

CREATE TYPE workflow_status AS ENUM ('draft', 'active', 'inactive', 'archived');
CREATE TYPE trigger_type AS ENUM ('manual', 'webhook', 'schedule', 'event', 'form');

-- ============================================================
-- WORKFLOWS
-- ============================================================

CREATE TABLE workflows (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    status          workflow_status NOT NULL DEFAULT 'draft',
    tags            TEXT[] NOT NULL DEFAULT '{}',

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
CREATE INDEX idx_workflows_tags ON workflows USING gin(tags);

-- ============================================================
-- WORKFLOW VERSIONS (immutable snapshots)
-- ============================================================

CREATE TABLE workflow_versions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version         INTEGER NOT NULL,                      -- 1, 2, 3...
    semver          VARCHAR(32),                           -- Optional: '1.2.0'
    description     TEXT,                                  -- changelog / release notes

    -- Full workflow definition as JSON (matches WorkflowDefinition struct)
    definition      JSONB NOT NULL,

    -- Extracted for querying without parsing JSON
    node_count      INTEGER NOT NULL DEFAULT 0,
    connection_count INTEGER NOT NULL DEFAULT 0,

    is_published    BOOLEAN NOT NULL DEFAULT FALSE,
    published_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    published_at    TIMESTAMPTZ,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (workflow_id, version)
);

CREATE INDEX idx_wf_versions_workflow ON workflow_versions(workflow_id);
CREATE INDEX idx_wf_versions_published ON workflow_versions(workflow_id) WHERE is_published = TRUE;

-- Add FK from workflows to active version (after versions table exists)
ALTER TABLE workflows
    ADD CONSTRAINT fk_active_version
    FOREIGN KEY (active_version_id)
    REFERENCES workflow_versions(id)
    ON DELETE SET NULL
    DEFERRABLE INITIALLY DEFERRED;

-- ============================================================
-- WORKFLOW TRIGGERS
-- ============================================================

CREATE TABLE workflow_triggers (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    trigger_type    trigger_type NOT NULL,
    name            VARCHAR(255) NOT NULL,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    config          JSONB NOT NULL DEFAULT '{}',

    -- Webhook-specific
    webhook_path    VARCHAR(512) UNIQUE,                   -- '/hooks/{uuid}'
    webhook_secret  TEXT,                                  -- HMAC secret

    -- Schedule-specific (cron)
    cron_expression VARCHAR(128),
    timezone        VARCHAR(64) DEFAULT 'UTC',
    next_run_at     TIMESTAMPTZ,
    last_run_at     TIMESTAMPTZ,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_triggers_workflow ON workflow_triggers(workflow_id);
CREATE INDEX idx_triggers_type ON workflow_triggers(trigger_type);
CREATE INDEX idx_triggers_next_run ON workflow_triggers(next_run_at)
    WHERE is_active = TRUE AND trigger_type = 'schedule';
CREATE INDEX idx_triggers_webhook ON workflow_triggers(webhook_path)
    WHERE webhook_path IS NOT NULL;

-- ============================================================
-- WORKFLOW SHARING (cross-tenant sharing, like n8n templates)
-- ============================================================

CREATE TABLE workflow_shares (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    shared_with     UUID REFERENCES organizations(id) ON DELETE CASCADE,   -- NULL = public
    permission      VARCHAR(32) NOT NULL DEFAULT 'read',   -- read | clone | execute
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workflow_id, shared_with)
);

CREATE INDEX idx_shares_workflow ON workflow_shares(workflow_id);
