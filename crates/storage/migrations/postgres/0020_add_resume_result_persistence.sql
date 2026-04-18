-- 0020: Resume result persistence (ADR-0009)
-- Layer: Execution
-- Spec: ADR-0009 (resume persistence schema)
--
-- Parity with Layer 1 migration 00000000000009_add_resume_persistence.sql.
-- `executions.input` already exists in Layer 2 (0011_executions.sql).
-- This migration extends `execution_nodes` (0012_execution_nodes.sql) with
-- full ActionResult variant storage.
--
-- Schema only; engine consumers land in chips B2 / B3 / B4.

-- Forward-compat guard (ADR-0009 §2): callers that see an unknown version
-- MUST surface a typed error, never fall back.
ALTER TABLE execution_nodes
    ADD COLUMN IF NOT EXISTS result_schema_version INTEGER NOT NULL DEFAULT 1;

-- Variant tag mirror ('Success' | 'Branch' | 'Route' | 'MultiOutput' |
-- 'Skip' | 'Wait' | 'Retry' | 'Break' | 'Continue' | 'Drop' | 'Terminate');
-- useful for SQL-side filtering without deserializing `result`.
ALTER TABLE execution_nodes
    ADD COLUMN IF NOT EXISTS result_kind TEXT;

-- Serialized ActionResult<Value>; NULL on legacy rows written before this
-- migration. B3 populates it.
ALTER TABLE execution_nodes
    ADD COLUMN IF NOT EXISTS result JSONB;
