-- 0020: Resume result persistence (ADR-0008)
-- Layer: Execution
-- Spec: ADR-0008 (resume persistence schema)
--
-- Parity with Postgres migration 0020 and Layer 1 migration 00000000000009.
-- `executions.input` already exists in Layer 2 (0011_executions.sql).
-- This migration extends `execution_nodes` (0012_execution_nodes.sql) with
-- full ActionResult variant storage.
--
-- Schema only; engine consumers land in chips B2 / B3 / B4.
--
-- SQLite dialect:
--   * INTEGER (not INTEGER NOT NULL DEFAULT 1 — SQLite ALTER cannot add
--     a NOT NULL column without a DEFAULT, and SQLite historically did not
--     accept column-level DEFAULT on ALTER ADD COLUMN; we set DEFAULT 1
--     which SQLite 3.35+ supports for ALTER.)
--   * TEXT stands in for JSON (application validates).

ALTER TABLE execution_nodes
    ADD COLUMN result_schema_version INTEGER NOT NULL DEFAULT 1;

ALTER TABLE execution_nodes
    ADD COLUMN result_kind TEXT;

ALTER TABLE execution_nodes
    ADD COLUMN result TEXT;
