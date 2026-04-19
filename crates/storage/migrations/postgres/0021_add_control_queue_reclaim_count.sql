-- 0021: Add reclaim_count to execution_control_queue (ADR-0017, ADR-0008 B1)
-- Layer: Execution
-- Spec: 12.2 (durable control plane), ADR-0017 (reclaim policy)

ALTER TABLE execution_control_queue
    ADD COLUMN reclaim_count BIGINT NOT NULL DEFAULT 0;

-- Partial index: only index Processing rows, which is what the reclaim
-- sweep queries. Keeps the index small on healthy queues where the vast
-- majority of rows are Completed / Failed.
CREATE INDEX idx_execution_control_queue_processing
    ON execution_control_queue (processed_at)
    WHERE status = 'Processing';
