-- 0026: Optional W3C Trace Context on control queue rows (M3.5)
-- JSON text; matches Postgres `JSONB` column semantics at the application layer.
-- The `json_valid` CHECK keeps SQLite parity with Postgres `JSONB`: malformed
-- payloads are rejected at INSERT/UPDATE time rather than blowing up the
-- engine's `serde_json::from_str` path on read.
-- Requires SQLite >= 3.38 (default in every Tier-1 backend Nebula supports).

ALTER TABLE execution_control_queue
    ADD COLUMN w3c_trace_context TEXT
    CHECK (w3c_trace_context IS NULL OR json_valid(w3c_trace_context));
