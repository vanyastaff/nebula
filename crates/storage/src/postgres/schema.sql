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
    lease_expires_at    TIMESTAMPTZ,
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
    processed_at    TIMESTAMPTZ,
    error_message   TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_control_queue_pending
    ON port_control_queue (status);

CREATE TABLE IF NOT EXISTS port_idempotency_marks (
    mark_key TEXT PRIMARY KEY
);

-- Durable idempotent-replay response cache (ADR-0048). `cache_key` is
-- already tenant-namespaced by the caller; first writer wins via
-- INSERT ... ON CONFLICT DO NOTHING. `expires_at` drives the sweep.
CREATE TABLE IF NOT EXISTS port_idempotency_cache (
    cache_key   TEXT PRIMARY KEY,
    status      INTEGER NOT NULL,
    headers     BYTEA NOT NULL,
    body        BYTEA NOT NULL,
    fingerprint BYTEA NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_port_idempotency_cache_expiry
    ON port_idempotency_cache (expires_at);

-- Webhook activation lookup (ADR-0049): incoming POST /hooks/{slug} →
-- owning trigger. Scoped: a slug is unique per tenant, so resolution
-- never crosses a tenant boundary.
CREATE TABLE IF NOT EXISTS port_webhook_activations (
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    slug         TEXT NOT NULL,
    trigger_id   TEXT NOT NULL,
    active       BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (workspace_id, org_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_port_webhook_activations_trigger
    ON port_webhook_activations (workspace_id, org_id, trigger_id);
