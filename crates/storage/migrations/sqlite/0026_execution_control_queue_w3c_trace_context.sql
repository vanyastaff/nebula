-- 0026: Optional W3C Trace Context on control queue rows (M3.5)
-- JSON text; matches Postgres `JSONB` column semantics at the application layer.

ALTER TABLE execution_control_queue
    ADD COLUMN w3c_trace_context TEXT;
