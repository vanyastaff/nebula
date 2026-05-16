-- Port-scoped SQLite schema for the execution-core contract.
--
-- This is the adapter's own minimal schema for the spec-16 execution
-- aggregate + idempotency guard. It deliberately does NOT FK into the
-- identity zoo (users/orgs/workspaces) — those tables and their migration
-- tree are owned by the identity stores; the execution core is independent
-- so single-process / :memory: deployments need no identity seeding.
--
-- `workspace_id` / `org_id` are plain TEXT scope columns: every scoped
-- query is `WHERE workspace_id = ? AND org_id = ?`, which is the row-level
-- tenant-isolation predicate the conformance suite asserts uniformly across
-- backends.

CREATE TABLE IF NOT EXISTS port_executions (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    workflow_id         TEXT NOT NULL,
    status              TEXT NOT NULL,
    state               TEXT NOT NULL,            -- opaque JSON
    version             INTEGER NOT NULL DEFAULT 0,
    lease_holder        TEXT,
    lease_expires_at_ms INTEGER,                  -- ms since epoch, NULL = no lease
    fencing_generation  INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_port_executions_scope
    ON port_executions (workspace_id, org_id);

CREATE INDEX IF NOT EXISTS idx_port_executions_workflow
    ON port_executions (workspace_id, org_id, workflow_id);

-- Append-only journal. `seq` is assigned per-execution monotonically.
CREATE TABLE IF NOT EXISTS port_execution_journal (
    execution_id  TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    payload       TEXT NOT NULL,                  -- opaque JSON
    PRIMARY KEY (execution_id, seq)
);

-- Control-queue outbox. `id` is the raw 16-byte ULID (BLOB), NOT the
-- UTF-8 of the ULID string — the legacy string-encoding hack is gone.
CREATE TABLE IF NOT EXISTS port_control_queue (
    id              BLOB PRIMARY KEY,             -- 16-byte ULID
    execution_id    TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    org_id          TEXT NOT NULL,
    command         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'Pending',
    w3c_traceparent TEXT,
    reclaim_count   INTEGER NOT NULL DEFAULT 0,
    processed_by    BLOB,
    processed_at_ms INTEGER,
    error_message   TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_control_queue_pending
    ON port_control_queue (status);

-- Per-attempt idempotency guard. The key already carries the scope so a
-- cross-tenant probe cannot collide with another tenant's mark.
CREATE TABLE IF NOT EXISTS port_idempotency_marks (
    mark_key TEXT PRIMARY KEY
);
