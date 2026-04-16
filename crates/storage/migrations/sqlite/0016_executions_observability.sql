-- 0016: Observability + coordination columns on executions
-- Spec: 18 (observability-stack), 17 (multi-process-coordination)

ALTER TABLE executions ADD COLUMN trace_id BLOB;
ALTER TABLE executions ADD COLUMN takeover_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_executions_trace
    ON executions (trace_id)
    WHERE trace_id IS NOT NULL;
