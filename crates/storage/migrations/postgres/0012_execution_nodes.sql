-- 0012: Execution nodes and pending signals
-- Layer: Execution
-- Spec: 16 (storage-schema), 09 (retry), 14 (stateful-actions)
--
-- Per-attempt node details: replaces the old `node_outputs` table.
-- Carries status, retry tracking, stateful action state, idempotency key,
-- and suspend/wake metadata.

CREATE TABLE execution_nodes (
    id                     BYTEA PRIMARY KEY,        -- att_ ULID
    execution_id           BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    logical_node_id        TEXT NOT NULL,             -- NodeKey from workflow definition
    attempt                INT NOT NULL,              -- 1, 2, 3, ... per retry

    -- Status
    status                 TEXT NOT NULL,             -- 'Running'/'Succeeded'/'Failed'/'Cancelled'/'PendingRetry'/'Suspended'
    started_at             TIMESTAMPTZ,
    finished_at            TIMESTAMPTZ,

    -- Input/output
    input                  JSONB,
    output                 JSONB,

    -- Error tracking (spec 09)
    error_kind             TEXT,                      -- 'Transient'/'Permanent'/'Cancelled'/'Fatal'/'Timeout'
    error_message          TEXT,
    error_retry_hint_ms    BIGINT,                   -- from TransientWithHint

    -- Idempotency (spec 15): {exec_id}:{logical_node_id}:{attempt}
    idempotency_key        TEXT NOT NULL,

    -- Retry / suspend tracking (spec 09, 14)
    wake_at                TIMESTAMPTZ,              -- NULL unless PendingRetry or Suspended with Timer
    wake_signal_name       TEXT,                     -- NULL unless Suspended with Signal

    -- StatefulAction state (spec 14); NULL for stateless
    state                  JSONB,                    -- inline state <= 1 MB
    state_blob_ref         BYTEA,                    -- reference for larger state (v1.5)
    state_schema_hash      BYTEA,                    -- for schema migration detection
    iteration_count        INT NOT NULL DEFAULT 0,

    -- Cancel escalation
    escalated              BOOLEAN NOT NULL DEFAULT FALSE,

    -- CAS
    version                BIGINT NOT NULL DEFAULT 0,

    UNIQUE (execution_id, logical_node_id, attempt),
    UNIQUE (idempotency_key)
);

-- List nodes for an execution
CREATE INDEX idx_execution_nodes_by_exec
    ON execution_nodes (execution_id, started_at);

-- Retry scheduler: find nodes ready to wake
CREATE INDEX idx_execution_nodes_pending_retry
    ON execution_nodes (wake_at)
    WHERE status = 'PendingRetry' AND wake_at IS NOT NULL;

-- Suspended nodes: timer or signal based
CREATE INDEX idx_execution_nodes_suspended
    ON execution_nodes (wake_at, wake_signal_name)
    WHERE status = 'Suspended';

-- ── Pending signals (for WaitUntil / Signal delivery) ──────

CREATE TABLE pending_signals (
    id                BYTEA PRIMARY KEY,             -- ULID
    node_attempt_id   BYTEA NOT NULL REFERENCES execution_nodes(id) ON DELETE CASCADE,
    signal_name       TEXT NOT NULL,
    payload           JSONB,
    received_at       TIMESTAMPTZ NOT NULL,
    consumed_at       TIMESTAMPTZ
);

CREATE INDEX idx_pending_signals_unconsumed
    ON pending_signals (node_attempt_id, signal_name)
    WHERE consumed_at IS NULL;
