CREATE TABLE IF NOT EXISTS executions (
    id          UUID        PRIMARY KEY,
    workflow_id UUID        NOT NULL,
    version     BIGINT      NOT NULL DEFAULT 1,
    state       JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_executions_workflow_id ON executions (workflow_id);
CREATE INDEX IF NOT EXISTS idx_executions_created_at ON executions (created_at DESC);
