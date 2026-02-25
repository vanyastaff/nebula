-- 013: Execution Node Runs & Logs

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

-- Retain only 90 days of execution logs
CREATE OR REPLACE FUNCTION cleanup_old_execution_logs() RETURNS void AS $$
BEGIN
    DELETE FROM execution_logs
    WHERE timestamp < NOW() - INTERVAL '90 days';
END;
$$ LANGUAGE plpgsql;
