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

-- Durable idempotent-replay response cache (ADR-0048). `cache_key` is
-- already tenant-namespaced by the caller (`{scope}:{key}`); first writer
-- wins via INSERT OR IGNORE. `expires_at_ms` drives the eviction sweep.
CREATE TABLE IF NOT EXISTS port_idempotency_cache (
    cache_key     TEXT PRIMARY KEY,
    status        INTEGER NOT NULL,
    headers       BLOB NOT NULL,
    body          BLOB NOT NULL,
    fingerprint   BLOB NOT NULL,
    expires_at    TEXT NOT NULL,            -- RFC 3339, returned verbatim
    expires_at_ms INTEGER NOT NULL          -- ms since epoch, sweep predicate
);

CREATE INDEX IF NOT EXISTS idx_port_idempotency_cache_expiry
    ON port_idempotency_cache (expires_at_ms);

-- Webhook activation lookup (ADR-0049): incoming POST /hooks/{slug} →
-- owning trigger, without scanning trigger configs. Scoped: a slug is
-- unique per tenant, so resolution never crosses a tenant boundary.
CREATE TABLE IF NOT EXISTS port_webhook_activations (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    slug         TEXT NOT NULL,
    trigger_id   TEXT NOT NULL,
    active       INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (workspace_id, org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_port_webhook_activations_trigger
    ON port_webhook_activations (workspace_id, org_id, trigger_id);

-- Spec-16 workflow split: the workflow row (id / slug / soft-delete /
-- CAS version) is separate from its versions. Scoped: every query is
-- `WHERE workspace_id = ? AND org_id = ?`, so a cross-tenant probe is
-- indistinguishable from a missing row (no existence oracle).
CREATE TABLE IF NOT EXISTS port_workflows (
    id           TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0,
    slug         TEXT NOT NULL,
    deleted      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, org_id, id)
);

-- A slug is unique per workspace among *active* rows only; a soft-deleted
-- row must not block re-creating the slug, so the uniqueness predicate is
-- partial on `deleted = 0`.
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_workflows_active_slug
    ON port_workflows (workspace_id, org_id, slug) WHERE deleted = 0;

-- Workflow versions. `definition` is opaque JSON owned by the workflow
-- compiler. `published` marks the served version; `pinned` excludes a
-- version from automatic GC.
CREATE TABLE IF NOT EXISTS port_workflow_versions (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    workflow_id  TEXT NOT NULL,
    number       INTEGER NOT NULL,
    published    INTEGER NOT NULL DEFAULT 0,
    pinned       INTEGER NOT NULL DEFAULT 0,
    definition   TEXT NOT NULL,            -- opaque JSON
    PRIMARY KEY (workspace_id, org_id, workflow_id, number)
);

CREATE INDEX IF NOT EXISTS idx_port_workflow_versions_published
    ON port_workflow_versions (workspace_id, org_id, workflow_id, published);
