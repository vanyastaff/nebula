-- ============================================================
-- 006: Executions (runtime state)
-- ============================================================

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

-- ============================================================
-- NODE RUNS (per-node execution results)
-- ============================================================

CREATE TABLE execution_node_runs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    execution_id    UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_id         VARCHAR(255) NOT NULL,                 -- matches NodeDefinition.id in workflow
    action_id       VARCHAR(255) NOT NULL,                 -- matches ActionId

    status          node_run_status NOT NULL DEFAULT 'pending',
    attempt         INTEGER NOT NULL DEFAULT 1,            -- retry count for this node

    -- Data
    input_data      JSONB,                                 -- resolved parameters
    output_data     JSONB,                                 -- action result

    -- Timing
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    duration_ms     INTEGER GENERATED ALWAYS AS (
                        EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000
                    ) STORED,

    -- Error
    error_message   TEXT,
    error_type      VARCHAR(128),                          -- 'timeout', 'validation', 'runtime'

    -- Resource usage
    memory_used_kb  INTEGER,
    cpu_time_ms     INTEGER,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_node_runs_execution ON execution_node_runs(execution_id);
CREATE INDEX idx_node_runs_status ON execution_node_runs(execution_id, status);

-- ============================================================
-- EXECUTION LOGS (structured log lines per execution)
-- ============================================================

CREATE TYPE log_level AS ENUM ('trace', 'debug', 'info', 'warn', 'error');

CREATE TABLE execution_logs (
    id              BIGSERIAL PRIMARY KEY,
    execution_id    UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_run_id     UUID REFERENCES execution_node_runs(id) ON DELETE CASCADE,
    level           log_level NOT NULL DEFAULT 'info',
    message         TEXT NOT NULL,
    data            JSONB,                                 -- structured context
    timestamp       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_exec_logs_execution ON execution_logs(execution_id, timestamp DESC);
CREATE INDEX idx_exec_logs_level ON execution_logs(execution_id, level);

-- ============================================================
-- IDEMPOTENCY KEYS
-- ============================================================

CREATE TABLE idempotency_keys (
    key             VARCHAR(512) PRIMARY KEY,
    execution_id    UUID REFERENCES executions(id) ON DELETE CASCADE,
    result          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_idem_keys_expires ON idempotency_keys(expires_at);

-- ============================================================
-- EXECUTION APPROVALS (human-in-the-loop, waiting state)
-- ============================================================

CREATE TYPE approval_status AS ENUM ('pending', 'approved', 'rejected', 'expired');

CREATE TABLE execution_approvals (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    execution_id    UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_run_id     UUID REFERENCES execution_node_runs(id) ON DELETE CASCADE,
    assignee_id     UUID REFERENCES users(id) ON DELETE SET NULL,
    status          approval_status NOT NULL DEFAULT 'pending',
    message         TEXT,
    response_note   TEXT,
    responded_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    responded_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_approvals_execution ON execution_approvals(execution_id);
CREATE INDEX idx_approvals_assignee ON execution_approvals(assignee_id) WHERE status = 'pending';
