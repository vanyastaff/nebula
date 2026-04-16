-- 0015: Slug history and audit log
-- Layer: Audit
-- Spec: 16 (storage-schema), 07 (slug-contract), 18 (observability)

CREATE TABLE slug_history (
    kind            TEXT NOT NULL,
    scope_id        BLOB,
    old_slug        TEXT NOT NULL,
    resource_id     BLOB NOT NULL,
    renamed_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    PRIMARY KEY (kind, scope_id, old_slug)
);

CREATE INDEX idx_slug_history_expiry
    ON slug_history (expires_at);

CREATE TABLE audit_log (
    id               BLOB PRIMARY KEY,
    org_id           BLOB NOT NULL,
    workspace_id     BLOB,
    actor_kind       TEXT NOT NULL,
    actor_id         BLOB,
    action           TEXT NOT NULL,
    target_kind      TEXT,
    target_id        BLOB,
    details          TEXT,                            -- JSON
    ip_address       TEXT,
    user_agent       TEXT,
    emitted_at       TEXT NOT NULL
);

CREATE INDEX idx_audit_log_by_org
    ON audit_log (org_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_actor
    ON audit_log (actor_kind, actor_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_action
    ON audit_log (action, emitted_at DESC);
