-- 0013: Execution journal and control queue
-- Layer: Execution
-- Spec: 16 (storage-schema), 12.2 (durable control plane)

-- ── Execution journal (append-only audit trail) ────────────
-- No UPDATE or DELETE in runtime code.
-- Retained via cascade when parent execution is deleted.

CREATE TABLE execution_journal (
    id              BYTEA PRIMARY KEY,               -- ULID, monotonic for ordering
    execution_id    BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    sequence        BIGINT NOT NULL,                 -- per-execution monotonic counter
    event_type      TEXT NOT NULL,                   -- 'ExecutionStarted'/'NodeStarted'/'NodeFinished'/...
    node_attempt_id BYTEA,                           -- NULL for execution-level events
    payload         JSONB NOT NULL,
    emitted_at      TIMESTAMPTZ NOT NULL,

    UNIQUE (execution_id, sequence)
);

CREATE INDEX idx_execution_journal_by_exec
    ON execution_journal (execution_id, sequence);

-- ── Execution control queue (outbox pattern, spec 12.2) ────
-- Every cancel/run/resume signal is written here atomically
-- with the corresponding state transition. A dispatch worker
-- drains pending commands and forwards them to the engine.

CREATE TABLE execution_control_queue (
    id              BYTEA PRIMARY KEY,               -- ULID
    execution_id    BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    command         TEXT NOT NULL,                   -- 'Cancel'/'Terminate'/'Resume'/'Restart'
    issued_by       BYTEA,                           -- user or service account
    issued_at       TIMESTAMPTZ NOT NULL,
    status          TEXT NOT NULL,                   -- 'Pending'/'Processing'/'Completed'/'Failed'
    processed_at    TIMESTAMPTZ,
    processed_by    BYTEA,                           -- instance_id that processed
    error_message   TEXT
);

CREATE INDEX idx_execution_control_queue_pending
    ON execution_control_queue (execution_id, issued_at)
    WHERE status = 'Pending';
