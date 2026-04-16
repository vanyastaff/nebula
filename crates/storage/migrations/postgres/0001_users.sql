-- 0001: Users
-- Layer: Identity
-- Spec: 16 (storage-schema), 03 (identity-auth)

CREATE TABLE users (
    id                 BYTEA PRIMARY KEY,            -- usr_ ULID (16 bytes)
    email              TEXT NOT NULL,
    email_verified_at  TIMESTAMPTZ,
    display_name       TEXT NOT NULL,
    avatar_url         TEXT,
    password_hash      TEXT,                         -- argon2id encoded; NULL for OAuth-only
    created_at         TIMESTAMPTZ NOT NULL,
    last_login_at      TIMESTAMPTZ,
    locked_until       TIMESTAMPTZ,
    failed_login_count INT NOT NULL DEFAULT 0,
    mfa_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_secret         BYTEA,                        -- encrypted with master key
    version            BIGINT NOT NULL DEFAULT 0,    -- CAS
    deleted_at         TIMESTAMPTZ
);

-- Case-insensitive unique email among active users
CREATE UNIQUE INDEX idx_users_email_active
    ON users (LOWER(email))
    WHERE deleted_at IS NULL;

-- Find locked accounts for unlock job
CREATE INDEX idx_users_locked
    ON users (locked_until)
    WHERE locked_until IS NOT NULL;
