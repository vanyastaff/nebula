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

-- Webhook activation lookup (ADR-0049, extended ADR-0096 commit 1):
-- incoming POST /hooks/{slug} → owning trigger, without scanning trigger
-- configs. Scoped: a slug is unique per tenant, so resolution never
-- crosses a tenant boundary.
--
-- workflow_id   NULL  → not yet wired to a specific workflow
-- webhook_mode  'test'→ safe default; 'prod' routes to the durable engine
-- token_hash    all-zeros sentinel → no capability token assigned yet
CREATE TABLE IF NOT EXISTS port_webhook_activations (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    slug         TEXT NOT NULL,
    trigger_id   TEXT NOT NULL,
    active       INTEGER NOT NULL DEFAULT 1,
    workflow_id  TEXT,
    webhook_mode TEXT NOT NULL DEFAULT 'test'
                     CHECK (webhook_mode IN ('test', 'prod')),
    token_hash   BLOB NOT NULL
                     DEFAULT X'0000000000000000000000000000000000000000000000000000000000000000'
                     CHECK (length(token_hash) = 32),
    PRIMARY KEY (workspace_id, org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_port_webhook_activations_trigger
    ON port_webhook_activations (workspace_id, org_id, trigger_id);

-- Partial unique index: the zero sentinel is excluded so rows without an
-- assigned token do not collide with each other. A non-sentinel token_hash
-- identifies at most one activation row (system-surface lookup).
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_webhook_activations_token_hash
    ON port_webhook_activations (token_hash)
    WHERE token_hash <> X'0000000000000000000000000000000000000000000000000000000000000000';

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

-- ── Identity zoo ──────────────────────────────────────────────────────────
--
-- Port-scoped TEXT-id form of the spec-16 identity aggregates (column sets
-- mirror migrations/postgres/{0001,0003,0004,0005,0009,0010,0014,0015,0019}
-- but as the adapter's own bare schema — no BYTEA, no cross-table FKs).
-- Uniqueness among *active* rows is a partial index `WHERE deleted_at IS
-- NULL`, so a soft-deleted row frees its email/slug. JSON-bearing columns
-- are opaque TEXT; binary columns are BLOB.

CREATE TABLE IF NOT EXISTS port_users (
    id                 TEXT PRIMARY KEY,
    email              TEXT NOT NULL,
    email_verified_at  TEXT,
    display_name       TEXT NOT NULL,
    avatar_url         TEXT,
    password_hash      TEXT,
    created_at         TEXT NOT NULL,
    last_login_at      TEXT,
    locked_until       TEXT,
    failed_login_count INTEGER NOT NULL DEFAULT 0,
    mfa_enabled        INTEGER NOT NULL DEFAULT 0,
    mfa_secret         BLOB,
    version            INTEGER NOT NULL DEFAULT 0,
    deleted_at         TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_users_active_email
    ON port_users (lower(email)) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS port_orgs (
    id            TEXT PRIMARY KEY,
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    plan          TEXT NOT NULL,
    billing_email TEXT,
    settings      TEXT NOT NULL DEFAULT '{}',   -- opaque JSON
    version       INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_orgs_active_slug
    ON port_orgs (slug) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS port_workspaces (
    id            TEXT NOT NULL,
    org_id        TEXT NOT NULL,
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    description   TEXT,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    is_default    INTEGER NOT NULL DEFAULT 0,
    settings      TEXT NOT NULL DEFAULT '{}',   -- opaque JSON
    version       INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    PRIMARY KEY (org_id, id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_workspaces_active_slug
    ON port_workspaces (org_id, slug) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS port_memberships (
    scope_kind     TEXT NOT NULL,
    scope_id       TEXT NOT NULL,
    principal_kind TEXT NOT NULL,
    principal_id   TEXT NOT NULL,
    role           TEXT NOT NULL,
    added_at       TEXT NOT NULL,
    added_by       TEXT,
    PRIMARY KEY (scope_kind, scope_id, principal_kind, principal_id)
);

CREATE TABLE IF NOT EXISTS port_resources (
    id            TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    org_id        TEXT NOT NULL,                -- port scope (no migration column)
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    config        TEXT NOT NULL,                -- opaque JSON
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    PRIMARY KEY (workspace_id, org_id, id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_resources_active_slug
    ON port_resources (workspace_id, org_id, slug) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS port_triggers (
    id            TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    org_id        TEXT NOT NULL,                -- port scope (no migration column)
    workflow_id   TEXT NOT NULL,
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    config        TEXT NOT NULL,                -- opaque JSON
    state         TEXT NOT NULL,
    run_as        TEXT,
    webhook_path  TEXT,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    PRIMARY KEY (workspace_id, org_id, id)
);

CREATE TABLE IF NOT EXISTS port_quotas (
    org_id                       TEXT PRIMARY KEY,
    plan                         TEXT NOT NULL,
    concurrent_executions_limit  INTEGER NOT NULL,
    executions_per_month_limit   INTEGER,
    active_workflows_limit       INTEGER,
    concurrent_executions        INTEGER NOT NULL DEFAULT 0,
    executions_this_month        INTEGER NOT NULL DEFAULT 0,
    month_reset_at               TEXT NOT NULL,
    updated_at                   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS port_audit_log (
    id           TEXT PRIMARY KEY,
    org_id       TEXT NOT NULL,
    workspace_id TEXT,
    actor_kind   TEXT NOT NULL,
    actor_id     TEXT,
    action       TEXT NOT NULL,
    target_kind  TEXT,
    target_id    TEXT,
    details      TEXT,                          -- opaque JSON, nullable
    ip_address   TEXT,
    user_agent   TEXT,
    emitted_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_port_audit_log_org
    ON port_audit_log (org_id, emitted_at);

CREATE TABLE IF NOT EXISTS port_blobs (
    id           TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    execution_id TEXT,
    kind         TEXT NOT NULL,
    content_type TEXT,
    size_bytes   INTEGER NOT NULL,
    checksum     BLOB,
    storage_mode TEXT NOT NULL,
    data         BLOB,
    external_ref TEXT,
    metadata     TEXT,                          -- opaque JSON, nullable
    created_at   TEXT NOT NULL,
    expires_at   TEXT,
    PRIMARY KEY (workspace_id, id)
);

-- Capability-routed job-dispatch queue.  `id` is the raw 16-byte ULID
-- (BLOB).  `required_plugins` is a JSON array of PluginKey strings; the
-- routing predicate is `required_plugins ⊆ available_plugins` (superset
-- check via NOT EXISTS + json_each).  `required_plugin_key` is the
-- denormalised primary/pre-filter key (index target).
-- `processed_at_ms` is epoch-millis (INTEGER) for parity with
-- `port_control_queue` and the reclaim arithmetic.
CREATE TABLE IF NOT EXISTS port_job_dispatch_queue (
    id                  BLOB PRIMARY KEY,       -- 16-byte ULID
    execution_id        TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    command             TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'Pending',
    payload             TEXT NOT NULL DEFAULT '{}',  -- opaque JSON
    event_id            TEXT,
    target_flavor_sha   TEXT NOT NULL DEFAULT '',
    required_plugin_key TEXT NOT NULL,
    required_plugins    TEXT NOT NULL DEFAULT '[]',  -- JSON array of PluginKey strings
    w3c_traceparent     TEXT,
    reclaim_count       INTEGER NOT NULL DEFAULT 0,
    processed_by        BLOB,
    processed_at_ms     INTEGER,
    error_message       TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_job_dispatch_queue_status_key
    ON port_job_dispatch_queue (status, required_plugin_key);

-- Trigger-dedup inbox.  `PRIMARY KEY(workspace_id, org_id, trigger_id, event_id)` is
-- the CAS for first-writer-wins fan-out dedup, scoped per tenant so two tenants
-- sharing a trigger_id + event_id never collide.
CREATE TABLE IF NOT EXISTS port_trigger_dedup_inbox (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    trigger_id   TEXT NOT NULL,
    event_id     TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, org_id, trigger_id, event_id)
);
