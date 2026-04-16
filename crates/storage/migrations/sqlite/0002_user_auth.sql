-- 0002: User authentication artifacts
-- Layer: Identity
-- Spec: 16 (storage-schema), 03 (identity-auth)

-- ── OAuth links ────────────────────────────────────────────

CREATE TABLE oauth_links (
    user_id            BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider           TEXT NOT NULL,
    provider_user_id   TEXT NOT NULL,
    provider_email     TEXT,
    linked_at          TEXT NOT NULL,                 -- ISO 8601
    PRIMARY KEY (provider, provider_user_id)
);

CREATE INDEX idx_oauth_links_user ON oauth_links (user_id);

-- ── Sessions ───────────────────────────────────────────────

CREATE TABLE sessions (
    id               BLOB PRIMARY KEY,
    user_id          BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at       TEXT NOT NULL,
    last_active_at   TEXT NOT NULL,
    expires_at       TEXT NOT NULL,
    ip_address       TEXT,
    user_agent       TEXT,
    revoked_at       TEXT
);

CREATE INDEX idx_sessions_user_active
    ON sessions (user_id)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_sessions_cleanup
    ON sessions (expires_at)
    WHERE revoked_at IS NULL;

-- ── Personal access tokens ─────────────────────────────────

CREATE TABLE personal_access_tokens (
    id                BLOB PRIMARY KEY,
    principal_kind    TEXT NOT NULL,
    principal_id      BLOB NOT NULL,
    name              TEXT NOT NULL,
    prefix            TEXT NOT NULL,
    hash              BLOB NOT NULL,
    scopes            TEXT NOT NULL,                  -- JSON array
    created_at        TEXT NOT NULL,
    last_used_at      TEXT,
    expires_at        TEXT,
    revoked_at        TEXT
);

CREATE INDEX idx_pat_hash
    ON personal_access_tokens (hash)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_pat_principal
    ON personal_access_tokens (principal_kind, principal_id);

-- ── Verification tokens ────────────────────────────────────

CREATE TABLE verification_tokens (
    token_hash   BLOB PRIMARY KEY,
    user_id      BLOB NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    payload      TEXT,                               -- JSON
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    consumed_at  TEXT
);

CREATE INDEX idx_verification_user_kind
    ON verification_tokens (user_id, kind)
    WHERE consumed_at IS NULL;

CREATE INDEX idx_verification_cleanup
    ON verification_tokens (expires_at)
    WHERE consumed_at IS NULL;
