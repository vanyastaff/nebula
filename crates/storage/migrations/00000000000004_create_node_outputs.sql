CREATE TABLE IF NOT EXISTS node_outputs (
    execution_id UUID        NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_id      UUID        NOT NULL,
    attempt      INT         NOT NULL DEFAULT 1,
    output       JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (execution_id, node_id, attempt)
);
