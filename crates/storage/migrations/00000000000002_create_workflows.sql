-- Create the workflows table for workflow definitions (used by PostgresWorkflowRepo).
CREATE TABLE IF NOT EXISTS workflows (
    id          UUID        PRIMARY KEY,
    version     BIGINT      NOT NULL DEFAULT 0,
    definition  JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflows_created_at ON workflows (created_at);
