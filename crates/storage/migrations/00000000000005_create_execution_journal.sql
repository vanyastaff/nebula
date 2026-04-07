CREATE TABLE IF NOT EXISTS execution_journal (
    id           BIGSERIAL   PRIMARY KEY,
    execution_id UUID        NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    entry        JSONB       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_journal_execution ON execution_journal (execution_id, created_at);
