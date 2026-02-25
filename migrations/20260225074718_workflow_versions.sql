-- ============================================================
-- 010: Workflow Versions & Triggers
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
