-- Execution listing projection and keyset history.
--
-- `status` was written once ('Created') on insert and never updated by a
-- commit, so no history query could filter on it. From this migration on the
-- execution owner writes the listing projection (status, started_at,
-- finished_at) in the same statement as every state snapshot.
--
-- The backfill reads each row's own persisted state once: the status and
-- lifecycle timestamps the owner already wrote. Nothing is inferred from
-- another aggregate and no authority is granted. A state without a known
-- status keeps 'created'; a lifecycle timestamp that is not RFC 3339 stays
-- NULL instead of aborting the upgrade or poisoning later reads.
--
-- `created_at_us` (microseconds since the epoch) is the keyset sort key on
-- both backends; SQLite stores timestamps as text, so the integer is the one
-- ordering both agree on. Rows written before this migration keep their
-- millisecond-precision creation instant (SQLite date functions resolve
-- milliseconds); the adapter writes full microseconds from now on. SQLite
-- cannot drop a column default, so it stays; every adapter write sets the
-- column explicitly.

ALTER TABLE port_executions ADD COLUMN started_at TEXT;
ALTER TABLE port_executions ADD COLUMN finished_at TEXT;
ALTER TABLE port_executions ADD COLUMN created_at_us INTEGER NOT NULL DEFAULT 0;

UPDATE port_executions
SET status = CASE
        WHEN json_extract(state, '$.status') IN ('created', 'running', 'paused', 'cancelling',
                                                 'completed', 'failed', 'cancelled', 'timed_out')
            THEN json_extract(state, '$.status')
        ELSE 'created'
    END,
    started_at = CASE
        WHEN json_type(state, '$.started_at') = 'text'
             AND json_extract(state, '$.started_at') GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]*'
             AND julianday(json_extract(state, '$.started_at')) IS NOT NULL
            THEN json_extract(state, '$.started_at')
    END,
    finished_at = CASE
        WHEN json_extract(state, '$.status') IN ('completed', 'failed', 'cancelled', 'timed_out')
             AND json_type(state, '$.completed_at') = 'text'
             AND json_extract(state, '$.completed_at') GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]*'
             AND julianday(json_extract(state, '$.completed_at')) IS NOT NULL
            THEN json_extract(state, '$.completed_at')
    END,
    created_at_us = CAST(strftime('%s', created_at) AS INTEGER) * 1000000
        + CAST(substr(strftime('%f', created_at), 4) AS INTEGER) * 1000;

DROP INDEX IF EXISTS idx_port_executions_scope;
DROP INDEX IF EXISTS idx_port_executions_workflow;

CREATE INDEX idx_port_executions_history
    ON port_executions (workspace_id, org_id, created_at_us DESC, id DESC);

CREATE INDEX idx_port_executions_workflow_history
    ON port_executions (workspace_id, org_id, workflow_id, created_at_us DESC, id DESC);

CREATE INDEX idx_port_executions_active
    ON port_executions (status)
    WHERE status IN ('created', 'running', 'paused', 'cancelling');
