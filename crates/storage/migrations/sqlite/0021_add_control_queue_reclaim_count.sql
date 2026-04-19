-- 0021: Add reclaim_count to execution_control_queue (ADR-0017, ADR-0008 B1)
-- Layer: Execution
-- Spec: 12.2 (durable control plane), ADR-0017 (reclaim policy)

ALTER TABLE execution_control_queue
    ADD COLUMN reclaim_count INTEGER NOT NULL DEFAULT 0;

-- Composite index for the reclaim sweep query:
--   WHERE status = 'Processing' AND processed_at < ?
-- SQLite cannot express `WHERE status = 'Processing'` as a partial-index
-- predicate with a parameterised timestamp, so we index on the pair and
-- accept that the Pending / Completed / Failed rows are also covered.
CREATE INDEX idx_execution_control_queue_processing
    ON execution_control_queue (status, processed_at);
