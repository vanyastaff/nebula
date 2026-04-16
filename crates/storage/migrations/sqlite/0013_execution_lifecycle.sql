-- 0013: Execution journal and control queue
-- Layer: Execution
-- Spec: 16 (storage-schema), 12.2 (durable control plane)

CREATE TABLE execution_journal (
    id              BLOB PRIMARY KEY,
    execution_id    BLOB NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    sequence        INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    node_attempt_id BLOB,
    payload         TEXT NOT NULL,                   -- JSON
    emitted_at      TEXT NOT NULL,

    UNIQUE (execution_id, sequence)
);

CREATE INDEX idx_execution_journal_by_exec
    ON execution_journal (execution_id, sequence);

CREATE TABLE execution_control_queue (
    id              BLOB PRIMARY KEY,
    execution_id    BLOB NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    command         TEXT NOT NULL,
    issued_by       BLOB,
    issued_at       TEXT NOT NULL,
    status          TEXT NOT NULL,
    processed_at    TEXT,
    processed_by    BLOB,
    error_message   TEXT
);

CREATE INDEX idx_execution_control_queue_pending
    ON execution_control_queue (execution_id, issued_at)
    WHERE status = 'Pending';
