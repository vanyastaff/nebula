-- 0001: Users
-- Layer: Identity
-- Spec: 16 (storage-schema), 03 (identity-auth)

CREATE TABLE users (
    id                 BLOB PRIMARY KEY,             -- usr_ ULID (16 bytes)
    email              TEXT NOT NULL,
    email_verified_at  TEXT,                          -- ISO 8601
    display_name       TEXT NOT NULL,
    avatar_url         TEXT,
    password_hash      TEXT,                          -- argon2id encoded; NULL for OAuth-only
    created_at         TEXT NOT NULL,                 -- ISO 8601
    last_login_at      TEXT,
    locked_until       TEXT,
    failed_login_count INTEGER NOT NULL DEFAULT 0,
    mfa_enabled        INTEGER NOT NULL DEFAULT 0,   -- 0/1 boolean
    mfa_secret         BLOB,                         -- encrypted with master key
    version            INTEGER NOT NULL DEFAULT 0,   -- CAS
    deleted_at         TEXT
);

-- Case-insensitive unique email among active users
CREATE UNIQUE INDEX idx_users_email_active
    ON users (LOWER(email))
    WHERE deleted_at IS NULL;

-- Find locked accounts for unlock job
CREATE INDEX idx_users_locked
    ON users (locked_until)
    WHERE locked_until IS NOT NULL;
