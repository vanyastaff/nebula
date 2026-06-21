-- Port-scoped Postgres schema for the execution-core contract.
--
-- The adapter's own minimal schema for the spec-16 execution aggregate +
-- idempotency guard. It does NOT FK into the identity zoo
-- (users/orgs/workspaces) — those tables and their migration tree are
-- owned by the identity stores; the execution core is independent so a
-- bare database needs no identity seeding for conformance.
--
-- `workspace_id` / `org_id` are plain TEXT scope columns: every scoped
-- query is `WHERE workspace_id = $ AND org_id = $`, the row-level tenant
-- isolation predicate the conformance suite asserts uniformly across
-- backends. Tables are prefixed `port_` so they never collide with the
-- legacy spec-16 tables in the structured migration tree.

CREATE TABLE IF NOT EXISTS port_executions (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    workflow_id         TEXT NOT NULL,
    status              TEXT NOT NULL,
    state               JSONB NOT NULL,
    version             BIGINT NOT NULL DEFAULT 0,
    lease_holder        TEXT,
    lease_expires_at_ms BIGINT,                    -- ms since epoch, NULL = no lease
    fencing_generation  BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_port_executions_scope
    ON port_executions (workspace_id, org_id);

CREATE INDEX IF NOT EXISTS idx_port_executions_workflow
    ON port_executions (workspace_id, org_id, workflow_id);

CREATE TABLE IF NOT EXISTS port_execution_journal (
    execution_id  TEXT NOT NULL,
    seq           BIGINT NOT NULL,
    payload       JSONB NOT NULL,
    PRIMARY KEY (execution_id, seq)
);

-- Control-queue outbox. `id` is the raw 16-byte ULID (BYTEA), NOT the
-- UTF-8 of the ULID string — the legacy string-encoding hack is gone.
-- `processed_at_ms` is epoch-millis (BIGINT) for parity with the SQLite
-- dialect and the Rust reclaim arithmetic in `postgres/control_queue.rs`.
CREATE TABLE IF NOT EXISTS port_control_queue (
    id              BYTEA PRIMARY KEY,
    execution_id    TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    org_id          TEXT NOT NULL,
    command         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'Pending',
    w3c_traceparent TEXT,
    reclaim_count   INTEGER NOT NULL DEFAULT 0,
    processed_by    BYTEA,
    processed_at_ms BIGINT,
    error_message   TEXT,
    -- ADR-0099 W-S3a: kind-aware resume targeting (serialized ResumeTarget
    -- JSON; NULL when None). NULL on legacy rows → deserialized as None.
    resume_target   TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_control_queue_pending
    ON port_control_queue (status);

CREATE TABLE IF NOT EXISTS port_idempotency_marks (
    mark_key TEXT PRIMARY KEY
);

-- Durable idempotent-replay response cache (ADR-0048). `cache_key` is
-- already tenant-namespaced by the caller; first writer wins via
-- INSERT ... ON CONFLICT DO NOTHING. `expires_at_ms` drives the sweep.
CREATE TABLE IF NOT EXISTS port_idempotency_cache (
    cache_key     TEXT PRIMARY KEY,
    status        INTEGER NOT NULL,
    headers       BYTEA NOT NULL,
    body          BYTEA NOT NULL,
    fingerprint   BYTEA NOT NULL,
    expires_at    TEXT NOT NULL,            -- RFC 3339, returned verbatim
    expires_at_ms BIGINT NOT NULL           -- ms since epoch, sweep predicate
);

CREATE INDEX IF NOT EXISTS idx_port_idempotency_cache_expiry
    ON port_idempotency_cache (expires_at_ms);

-- Webhook activation lookup: incoming webhook → owning trigger. Scoped per
-- tenant. ADR-0096 added workflow_id / webhook_mode / token_hash (mirrors
-- migration 0032 + sqlite/schema.sql; this DDL provisions the conformance
-- pool, which does NOT layer the migrations — keep all four SQL files in sync).
-- token_hash resolves the capability-token path (resolve_by_token); the
-- all-zeros sentinel ("no token assigned") is excluded from the unique index.
CREATE TABLE IF NOT EXISTS port_webhook_activations (
    workspace_id    TEXT    NOT NULL,
    org_id          TEXT    NOT NULL,
    slug            TEXT    NOT NULL,
    trigger_id      TEXT    NOT NULL,
    active          BOOLEAN NOT NULL DEFAULT TRUE,
    workflow_id     TEXT,
    webhook_mode    TEXT    NOT NULL DEFAULT 'test'
        CHECK (webhook_mode IN ('test', 'prod')),
    token_hash      BYTEA   NOT NULL
        DEFAULT decode(repeat('00', 32), 'hex')
        CHECK (octet_length(token_hash) = 32),
    spec_trigger_id TEXT,    -- ADR-0101 L1 spec link: port_triggers PK (trg_ prefix), NULL on legacy rows
    PRIMARY KEY (workspace_id, org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_port_webhook_activations_trigger
    ON port_webhook_activations (workspace_id, org_id, trigger_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_port_webhook_activations_token_hash
    ON port_webhook_activations (token_hash)
    WHERE token_hash <> decode(repeat('00', 32), 'hex');

-- Spec-16 workflow split: the workflow row (id / slug / soft-delete /
-- CAS version) is separate from its versions. Scoped: every query is
-- `WHERE workspace_id = $ AND org_id = $`, so a cross-tenant probe is
-- indistinguishable from a missing row (no existence oracle).
CREATE TABLE IF NOT EXISTS port_workflows (
    id           TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    version      BIGINT NOT NULL DEFAULT 0,
    slug         TEXT NOT NULL,
    deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (workspace_id, org_id, id)
);

-- A slug is unique per workspace among *active* rows only; a soft-deleted
-- row must not block re-creating the slug, so the uniqueness predicate is
-- partial on `deleted = FALSE`.
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_workflows_active_slug
    ON port_workflows (workspace_id, org_id, slug) WHERE deleted = FALSE;

-- Workflow versions. `definition` is opaque JSON owned by the workflow
-- compiler. `published` marks the served version; `pinned` excludes a
-- version from automatic GC.
CREATE TABLE IF NOT EXISTS port_workflow_versions (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    workflow_id  TEXT NOT NULL,
    number       BIGINT NOT NULL,
    published    BOOLEAN NOT NULL DEFAULT FALSE,
    pinned       BOOLEAN NOT NULL DEFAULT FALSE,
    definition   JSONB NOT NULL,
    PRIMARY KEY (workspace_id, org_id, workflow_id, number)
);

CREATE INDEX IF NOT EXISTS idx_port_workflow_versions_published
    ON port_workflow_versions (workspace_id, org_id, workflow_id, published);

-- ── Identity zoo ──────────────────────────────────────────────────────────
--
-- Port-scoped TEXT-id form of the spec-16 identity aggregates (column sets
-- mirror migrations/postgres/{0001,0003,0004,0005,0009,0010,0014,0015,0019}
-- but as the adapter's own bare schema — no BYTEA ids, no cross-table FKs).
-- Uniqueness among *active* rows is a partial unique index `WHERE
-- deleted_at IS NULL`, so a soft-deleted row frees its email/slug. JSON
-- columns are `JSONB`; binary columns are `BYTEA`.

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
    failed_login_count BIGINT NOT NULL DEFAULT 0,
    mfa_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_secret         BYTEA,
    version            BIGINT NOT NULL DEFAULT 0,
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
    settings      JSONB NOT NULL DEFAULT '{}',
    version       BIGINT NOT NULL DEFAULT 0,
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
    is_default    BOOLEAN NOT NULL DEFAULT FALSE,
    settings      JSONB NOT NULL DEFAULT '{}',
    version       BIGINT NOT NULL DEFAULT 0,
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
    org_id        TEXT NOT NULL,
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    config        JSONB NOT NULL,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    version       BIGINT NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    PRIMARY KEY (workspace_id, org_id, id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_resources_active_slug
    ON port_resources (workspace_id, org_id, slug) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS port_triggers (
    id            TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    org_id        TEXT NOT NULL,
    workflow_id   TEXT NOT NULL,
    slug          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    kind          TEXT NOT NULL,
    config        JSONB NOT NULL,
    state         TEXT NOT NULL,
    run_as        TEXT,
    webhook_path  TEXT,
    created_at    TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    version       BIGINT NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    PRIMARY KEY (workspace_id, org_id, id)
);

CREATE TABLE IF NOT EXISTS port_quotas (
    org_id                       TEXT PRIMARY KEY,
    plan                         TEXT NOT NULL,
    concurrent_executions_limit  BIGINT NOT NULL,
    executions_per_month_limit   BIGINT,
    active_workflows_limit       BIGINT,
    concurrent_executions        BIGINT NOT NULL DEFAULT 0,
    executions_this_month        BIGINT NOT NULL DEFAULT 0,
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
    details      JSONB,
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
    size_bytes   BIGINT NOT NULL,
    checksum     BYTEA,
    storage_mode TEXT NOT NULL,
    data         BYTEA,
    external_ref TEXT,
    metadata     JSONB,
    created_at   TEXT NOT NULL,
    expires_at   TEXT,
    PRIMARY KEY (workspace_id, id)
);

-- Capability-routed job-dispatch queue.  `id` is the raw 16-byte ULID
-- (BYTEA).  `required_plugins` is a JSONB array of PluginKey strings.
-- Claim predicate:
--   `required_plugin_key = ANY($available)`            (B-tree pre-filter below)
--   AND `required_plugins <@ $available_jsonb`         (JSONB subset / superset check)
-- The pre-filter is sound — `required_plugins ⊇ {required_plugin_key}` (DTO
-- invariant) — so it never drops a row the subset check would accept.
-- PERF NOTE: the `<@` subset check is NOT GIN-accelerated.  The built-in
-- `jsonb_ops` GIN class indexes `@>` / existence / jsonpath, NOT `<@` (only the
-- `array_ops` class indexes `<@`, and only for native arrays).  So `<@` runs as
-- a filter over the rows the `(status, required_plugin_key)` B-tree returns —
-- acceptable pre-fleet.  Before production scale, move `required_plugins` to a
-- `<@`-indexable representation (text[] + `array_ops`, or a normalized
-- plugin-membership table).  Tracked as an ADR-0095 D1 follow-up.
-- `processed_at_ms` is epoch-millis (BIGINT) for parity with the
-- `port_control_queue` reclaim arithmetic.
CREATE TABLE IF NOT EXISTS port_job_dispatch_queue (
    id                  BYTEA PRIMARY KEY,
    execution_id        TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    command             TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'Pending',
    payload             JSONB NOT NULL DEFAULT '{}',
    event_id            TEXT,
    target_flavor_sha   TEXT NOT NULL DEFAULT '',
    required_plugin_key TEXT NOT NULL,
    required_plugins    JSONB NOT NULL DEFAULT '[]',
    w3c_traceparent     TEXT,
    reclaim_count       INTEGER NOT NULL DEFAULT 0,
    processed_by        BYTEA,
    processed_at_ms     BIGINT,
    error_message       TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_job_dispatch_queue_status_key
    ON port_job_dispatch_queue (status, required_plugin_key);

-- jsonb_ops GIN: does NOT accelerate the current `<@` subset claim (see the
-- PERF NOTE above); retained for future `@>` / existence membership queries.
-- The `<@`-indexable representation is the tracked pre-production follow-up.
CREATE INDEX IF NOT EXISTS idx_port_job_dispatch_queue_plugins
    ON port_job_dispatch_queue USING GIN (required_plugins);

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
