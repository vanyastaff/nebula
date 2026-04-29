-- 0011: Executions
-- Layer: Execution
-- Spec: 16 (storage-schema), 08 (cancellation), 17 (multi-process)
--
-- Status (1.0): table schema is shipped, but engine consumers are Layer 1
-- only — they go through the top-level `nebula_storage::ExecutionRepo`
-- (defined in `crates/storage/src/execution_repo.rs`), which uses its own
-- `lease_holder`/`lease_expires_at` columns added by common migration
-- `00000000000007_add_execution_leases.sql`. The `claimed_by`/`claimed_until`
-- columns and `idx_executions_pending_claim` / `idx_executions_stale_lease`
-- indexes defined below are Sprint E (1.1) scaffolding — they have no
-- engine consumers today. See ROADMAP "Out of scope for 1.0" → "Storage
-- Layer 2 / spec-16 multi-tenant row model (Sprint E)".

CREATE TABLE executions (
    id                     BLOB PRIMARY KEY,
    workspace_id           BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    org_id                 BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workflow_version_id    BLOB NOT NULL REFERENCES workflow_versions(id),
    status                 TEXT NOT NULL,
    source                 TEXT NOT NULL,             -- JSON
    input                  TEXT,
    output                 TEXT,
    vars                   TEXT,
    progress_summary       TEXT,

    created_at             TEXT NOT NULL,
    scheduled_at           TEXT,
    started_at             TEXT,
    finished_at            TEXT,

    claimed_by             BLOB,
    claimed_until          TEXT,

    cancel_requested_at    TEXT,
    cancel_requested_by    BLOB,
    cancel_reason          TEXT,
    escalated              INTEGER NOT NULL DEFAULT 0,

    restarted_from         BLOB REFERENCES executions(id),

    execution_timeout_at   TEXT,

    version                INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_executions_pending_claim
    ON executions (created_at)
    WHERE status IN ('Pending', 'Queued') AND claimed_until IS NULL;

CREATE INDEX idx_executions_stale_lease
    ON executions (claimed_until)
    WHERE status = 'Running' AND claimed_until IS NOT NULL;

CREATE INDEX idx_executions_workspace_list
    ON executions (workspace_id, created_at DESC);

CREATE INDEX idx_executions_by_version
    ON executions (workflow_version_id);

CREATE INDEX idx_executions_timeout_check
    ON executions (execution_timeout_at)
    WHERE status = 'Running' AND execution_timeout_at IS NOT NULL;

CREATE INDEX idx_executions_restart_chain
    ON executions (restarted_from)
    WHERE restarted_from IS NOT NULL;
