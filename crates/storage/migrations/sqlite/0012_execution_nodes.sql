-- 0012: Execution nodes and pending signals
-- Layer: Execution
-- Spec: 16 (storage-schema), 09 (retry), 14 (stateful-actions)

CREATE TABLE execution_nodes (
    id                     BLOB PRIMARY KEY,
    execution_id           BLOB NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    logical_node_id        TEXT NOT NULL,
    attempt                INTEGER NOT NULL,

    status                 TEXT NOT NULL,
    started_at             TEXT,
    finished_at            TEXT,

    input                  TEXT,                      -- JSON
    output                 TEXT,

    error_kind             TEXT,
    error_message          TEXT,
    error_retry_hint_ms    INTEGER,

    idempotency_key        TEXT NOT NULL,

    wake_at                TEXT,
    wake_signal_name       TEXT,

    state                  TEXT,                      -- JSON
    state_blob_ref         BLOB,
    state_schema_hash      BLOB,
    iteration_count        INTEGER NOT NULL DEFAULT 0,

    escalated              INTEGER NOT NULL DEFAULT 0,

    version                INTEGER NOT NULL DEFAULT 0,

    UNIQUE (execution_id, logical_node_id, attempt),
    UNIQUE (idempotency_key)
);

CREATE INDEX idx_execution_nodes_by_exec
    ON execution_nodes (execution_id, started_at);

CREATE INDEX idx_execution_nodes_pending_retry
    ON execution_nodes (wake_at)
    WHERE status = 'PendingRetry' AND wake_at IS NOT NULL;

CREATE INDEX idx_execution_nodes_suspended
    ON execution_nodes (wake_at, wake_signal_name)
    WHERE status = 'Suspended';

CREATE TABLE pending_signals (
    id                BLOB PRIMARY KEY,
    node_attempt_id   BLOB NOT NULL REFERENCES execution_nodes(id) ON DELETE CASCADE,
    signal_name       TEXT NOT NULL,
    payload           TEXT,                           -- JSON
    received_at       TEXT NOT NULL,
    consumed_at       TEXT
);

CREATE INDEX idx_pending_signals_unconsumed
    ON pending_signals (node_attempt_id, signal_name)
    WHERE consumed_at IS NULL;
