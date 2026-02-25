-- 012: Executions

CREATE TYPE execution_status AS ENUM (
    'queued',
    'running',
    'success',
    'failed',
    'cancelled',
    'timed_out',
    'waiting'   -- waiting for external event / approval
);

CREATE TYPE node_run_status AS ENUM (
    'pending',
    'running',
    'success',
    'failed',
    'skipped',
    'cancelled'
);

-- ============================================================
-- EXECUTIONS
-- ============================================================

CREATE TABLE executions (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id           UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    workflow_id         UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    workflow_version_id UUID NOT NULL REFERENCES workflow_versions(id),
    trigger_id          UUID REFERENCES workflow_triggers(id) ON DELETE SET NULL,

    status              execution_status NOT NULL DEFAULT 'queued',
    mode                VARCHAR(32) NOT NULL DEFAULT 'production',  -- production | test | debug

    -- Input / Output
    input_data          JSONB,                             -- trigger payload or manual input
    output_data         JSONB,                             -- final result of the workflow

    -- Execution context
    triggered_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    trigger_type        trigger_type,
    idempotency_key     VARCHAR(512) UNIQUE,               -- for dedup

    -- Timing
    queued_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    finished_at         TIMESTAMPTZ,
    duration_ms         INTEGER GENERATED ALWAYS AS (
                            EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000
                        ) STORED,

    -- Error
    error_message       TEXT,
    error_node_id       VARCHAR(255),                      -- which node caused the failure

    -- Retry info
    retry_count         INTEGER NOT NULL DEFAULT 0,
    retry_of            UUID REFERENCES executions(id),    -- if this is a retry
    parent_execution_id UUID REFERENCES executions(id),    -- for sub-workflows

    -- Worker assignment (for clustering)
    worker_id           UUID,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_executions_tenant ON executions(tenant_id);
CREATE INDEX idx_executions_workflow ON executions(workflow_id);
CREATE INDEX idx_executions_status ON executions(status);
CREATE INDEX idx_executions_queued ON executions(queued_at DESC) WHERE status = 'queued';
CREATE INDEX idx_executions_running ON executions(started_at) WHERE status = 'running';
CREATE INDEX idx_executions_worker ON executions(worker_id) WHERE worker_id IS NOT NULL;
CREATE INDEX idx_executions_idem ON executions(idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Partial index for active executions (critical performance path)
CREATE INDEX idx_executions_active
    ON executions(tenant_id, workflow_id, created_at DESC)
    WHERE status IN ('queued', 'running', 'waiting');
