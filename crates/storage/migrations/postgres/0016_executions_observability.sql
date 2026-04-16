-- 0016: Observability + coordination columns on executions
-- Spec: 18 (observability-stack), 17 (multi-process-coordination)
--
-- trace_id: OpenTelemetry trace correlation, persisted at execution creation.
-- takeover_count: circuit breaker for crash loops — orphan after 3+ takeovers.

ALTER TABLE executions ADD COLUMN trace_id BYTEA;
ALTER TABLE executions ADD COLUMN takeover_count INT NOT NULL DEFAULT 0;

CREATE INDEX idx_executions_trace
    ON executions (trace_id)
    WHERE trace_id IS NOT NULL;
