-- 0015: Slug history and audit log
-- Layer: Audit
-- Spec: 16 (storage-schema), 07 (slug-contract), 18 (observability)

-- ── Slug history (rename grace period) ─────────────────────

CREATE TABLE slug_history (
    kind            TEXT NOT NULL,                   -- 'org' / 'workspace' / 'workflow' / 'trigger' / ...
    scope_id        BYTEA,                           -- NULL for org, else parent id
    old_slug        TEXT NOT NULL,
    resource_id     BYTEA NOT NULL,                  -- target entity
    renamed_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (kind, scope_id, old_slug)
);

CREATE INDEX idx_slug_history_expiry
    ON slug_history (expires_at);

-- ── Audit log ──────────────────────────────────────────────
-- High-level security/compliance events, separate from execution_journal.
-- Retention: 90 days default, enterprise configurable.

CREATE TABLE audit_log (
    id               BYTEA PRIMARY KEY,              -- ULID
    org_id           BYTEA NOT NULL,
    workspace_id     BYTEA,                          -- NULL for org-level events
    actor_kind       TEXT NOT NULL,                  -- 'user' / 'service_account' / 'system'
    actor_id         BYTEA,                          -- nullable for system events
    action           TEXT NOT NULL,                  -- 'workflow.created' / 'credential.rotated' / 'user.invited' / ...
    target_kind      TEXT,
    target_id        BYTEA,
    details          JSONB,
    ip_address       INET,
    user_agent       TEXT,
    emitted_at       TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_audit_log_by_org
    ON audit_log (org_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_actor
    ON audit_log (actor_kind, actor_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_action
    ON audit_log (action, emitted_at DESC);
