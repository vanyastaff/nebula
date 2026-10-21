-- Finished executions in `finished_at` order, for the retention sweep that
-- deletes terminal executions past the configured age. Only terminal rows
-- are indexed; live executions never appear in the scan.
CREATE INDEX ix_executions__finished_at__terminal
    ON executions (finished_at)
    WHERE status IN ('completed', 'failed', 'cancelled', 'timed_out');
