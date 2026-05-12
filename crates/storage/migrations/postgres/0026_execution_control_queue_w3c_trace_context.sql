-- 0026: Optional W3C Trace Context on control queue rows (M3.5)
-- JSON shape matches `nebula_core::W3cTraceContext` (traceparent + optional tracestate).

ALTER TABLE execution_control_queue
    ADD COLUMN w3c_trace_context JSONB;
