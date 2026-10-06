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
-- NULL instead of aborting the upgrade.
--
-- `created_at_us` (microseconds since the epoch) is the keyset sort key on
-- both backends; SQLite stores timestamps as text, so the integer is the one
-- ordering both agree on. The default exists only for the backfill and is
-- dropped so a writer that predates this migration fails closed.

ALTER TABLE port_executions ADD COLUMN started_at TIMESTAMPTZ;
ALTER TABLE port_executions ADD COLUMN finished_at TIMESTAMPTZ;
ALTER TABLE port_executions ADD COLUMN created_at_us BIGINT NOT NULL DEFAULT 0;

UPDATE port_executions
SET status = CASE
        WHEN state->>'status' IN ('created', 'running', 'paused', 'cancelling',
                                  'completed', 'failed', 'cancelled', 'timed_out')
            THEN state->>'status'
        ELSE 'created'
    END,
    started_at = CASE
        WHEN jsonb_typeof(state->'started_at') = 'string'
             AND state->>'started_at' ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$'
            THEN (state->>'started_at')::timestamptz
    END,
    finished_at = CASE
        WHEN state->>'status' IN ('completed', 'failed', 'cancelled', 'timed_out')
             AND jsonb_typeof(state->'completed_at') = 'string'
             AND state->>'completed_at' ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$'
            THEN (state->>'completed_at')::timestamptz
    END,
    created_at_us = floor(extract(epoch FROM created_at)::numeric * 1000000)::bigint;

ALTER TABLE port_executions ALTER COLUMN created_at_us DROP DEFAULT;

DROP INDEX IF EXISTS idx_port_executions_scope;
DROP INDEX IF EXISTS idx_port_executions_workflow;

-- The id tiebreak compares bytes (`COLLATE "C"`), the order SQLite and the
-- reference adapter use; a locale collation would page differently.
CREATE INDEX idx_port_executions_history
    ON port_executions (workspace_id, org_id, created_at_us DESC, id COLLATE "C" DESC);

CREATE INDEX idx_port_executions_workflow_history
    ON port_executions (workspace_id, org_id, workflow_id, created_at_us DESC, id COLLATE "C" DESC);

CREATE INDEX idx_port_executions_active
    ON port_executions (status)
    WHERE status IN ('created', 'running', 'paused', 'cancelling');
