-- 0002: User authentication artifacts
-- Layer: Identity
-- Spec: 16 (storage-schema), 03 (identity-auth)

-- ── OAuth links ────────────────────────────────────────────

CREATE TABLE oauth_links (
    user_id            BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider           TEXT NOT NULL,                -- 'google' / 'github' / 'microsoft'
    provider_user_id   TEXT NOT NULL,
    provider_email     TEXT,
    linked_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (provider, provider_user_id)
);

CREATE INDEX idx_oauth_links_user ON oauth_links (user_id);

-- ── Sessions ───────────────────────────────────────────────

CREATE TABLE sessions (
    id               BYTEA PRIMARY KEY,              -- sess_ ULID
    user_id          BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at       TIMESTAMPTZ NOT NULL,
    last_active_at   TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    ip_address       INET,
    user_agent       TEXT,
    revoked_at       TIMESTAMPTZ
);

CREATE INDEX idx_sessions_user_active
    ON sessions (user_id)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_sessions_cleanup
    ON sessions (expires_at)
    WHERE revoked_at IS NULL;

-- ── Personal access tokens ─────────────────────────────────

CREATE TABLE personal_access_tokens (
    id                BYTEA PRIMARY KEY,             -- pat_ ULID
    principal_kind    TEXT NOT NULL,                  -- 'user' / 'service_account'
    principal_id      BYTEA NOT NULL,
    name              TEXT NOT NULL,
    prefix            TEXT NOT NULL,                  -- first 12 chars for display
    hash              BYTEA NOT NULL,                 -- sha256 of full token
    scopes            JSONB NOT NULL,                 -- [] = full, or ['read', 'workflows', ...]
    created_at        TIMESTAMPTZ NOT NULL,
    last_used_at      TIMESTAMPTZ,
    expires_at        TIMESTAMPTZ,
    revoked_at        TIMESTAMPTZ
);

CREATE INDEX idx_pat_hash
    ON personal_access_tokens (hash)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_pat_principal
    ON personal_access_tokens (principal_kind, principal_id);

-- ── Verification tokens ────────────────────────────────────

CREATE TABLE verification_tokens (
    token_hash   BYTEA PRIMARY KEY,                  -- sha256 of token value
    user_id      BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,                      -- 'email_verification' / 'password_reset' / 'org_invite' / 'mfa_recovery'
    payload      JSONB,                              -- kind-specific data
    created_at   TIMESTAMPTZ NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    consumed_at  TIMESTAMPTZ
);

CREATE INDEX idx_verification_user_kind
    ON verification_tokens (user_id, kind)
    WHERE consumed_at IS NULL;

CREATE INDEX idx_verification_cleanup
    ON verification_tokens (expires_at)
    WHERE consumed_at IS NULL;
