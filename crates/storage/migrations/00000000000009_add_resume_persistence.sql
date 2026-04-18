-- 0009: Resume persistence schema (ADR-0008)
-- Foundation for resume replay-completeness (issues #299, #311, #324, #336).
-- Schema only; engine consumers land in chips B2 (workflow input), B3 (result
-- writes), B4 (resume reconstruction).

-- Workflow trigger / start input — durable so resume can replay entry nodes
-- with the original payload (issue #311). Nullable: pre-migration executions
-- have no input; B2 introduces the write path.
ALTER TABLE executions
    ADD COLUMN IF NOT EXISTS input JSONB;

-- Full ActionResult variant per node attempt (issue #299).
--   * result_schema_version: forward-compat guard (ADR-0008 §2); callers that
--     see an unknown version MUST surface a typed error, never fall back.
--   * result_kind: variant tag mirror ('Success' | 'Branch' | 'Route' |
--     'MultiOutput' | 'Skip' | 'Wait' | 'Retry' | 'Break' | 'Continue' |
--     'Drop' | 'Terminate'); useful for SQL-side filtering without
--     deserializing `result`.
--   * result: serialized ActionResult<Value>; NULL on legacy rows written
--     before this migration. B3 populates it.
ALTER TABLE node_outputs
    ADD COLUMN IF NOT EXISTS result_schema_version INTEGER NOT NULL DEFAULT 1;

ALTER TABLE node_outputs
    ADD COLUMN IF NOT EXISTS result_kind TEXT;

ALTER TABLE node_outputs
    ADD COLUMN IF NOT EXISTS result JSONB;

-- Relax legacy `output` to NULLABLE so `save_node_result` can insert a row
-- that carries only the new variant columns. Existing rows retain their
-- primary-output value; new rows written by B3 via `save_node_result`
-- may omit it. `load_node_output` keeps working against rows that still
-- have `output` populated; resume-path readers call `load_node_result`.
ALTER TABLE node_outputs
    ALTER COLUMN output DROP NOT NULL;
