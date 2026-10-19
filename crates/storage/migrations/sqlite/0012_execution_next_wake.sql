-- The earliest durable timer of a parked node, projected from the state
-- snapshot on every commit like `status`, `started_at` and `finished_at`.
-- The timer sweep reads only rows whose timer is due through the partial
-- index instead of decoding every live execution. Existing live rows are
-- backfilled from their stored state, floored to the second; the sweep
-- re-checks the state, so an early row is skipped until its timer is due.
ALTER TABLE executions ADD COLUMN next_wake_at INTEGER;

UPDATE executions
SET next_wake_at = (
    SELECT MIN(CAST(strftime('%s', json_extract(n.value, '$.next_attempt_at')) AS INTEGER) * 1000000)
    FROM json_each(executions.state, '$.node_states') AS n
    WHERE json_extract(n.value, '$.state') = 'waiting'
      AND json_extract(n.value, '$.next_attempt_at') IS NOT NULL
)
WHERE status IN ('created', 'running', 'paused', 'cancelling');

CREATE INDEX ix_executions__next_wake_at_id
    ON executions (next_wake_at, id)
    WHERE next_wake_at IS NOT NULL;
