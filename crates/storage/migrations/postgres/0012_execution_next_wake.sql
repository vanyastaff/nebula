-- The earliest durable timer of a parked node, projected from the state
-- snapshot on every commit like `status`, `started_at` and `finished_at`.
-- The timer sweep reads only rows whose timer is due through the partial
-- index instead of decoding every live execution. Existing live rows are
-- backfilled from their stored state.
ALTER TABLE executions ADD COLUMN next_wake_at TIMESTAMPTZ;

UPDATE executions AS e
SET next_wake_at = due.wake
FROM (
    SELECT x.id, MIN((n.value ->> 'next_attempt_at')::timestamptz) AS wake
    FROM executions AS x,
         jsonb_each(
             CASE WHEN jsonb_typeof(x.state -> 'node_states') = 'object'
                  THEN x.state -> 'node_states'
                  ELSE '{}'::jsonb
             END
         ) AS n
    WHERE x.status IN ('created', 'running', 'paused', 'cancelling')
      AND n.value ->> 'state' = 'waiting'
      AND n.value ->> 'next_attempt_at' IS NOT NULL
    GROUP BY x.id
) AS due
WHERE e.id = due.id;

CREATE INDEX ix_executions__next_wake_at_id
    ON executions (next_wake_at, id COLLATE "C")
    WHERE next_wake_at IS NOT NULL;
