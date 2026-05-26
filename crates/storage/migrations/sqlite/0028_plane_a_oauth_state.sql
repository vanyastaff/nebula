-- 0028: Plane-A OAuth PKCE state
-- Layer: Identity
-- Spec: 03 (identity-auth)
--
-- SQLite parity of the Postgres migration. Timestamps are stored as
-- ISO 8601 TEXT per the project convention (see
-- 0002_user_auth.sql sqlite mirror). The primary key is TEXT because
-- the `state` value is a random url-safe string, not a ULID.

CREATE TABLE plane_a_oauth_states (
    state          TEXT PRIMARY KEY,
    provider       TEXT NOT NULL,
    code_verifier  TEXT NOT NULL,
    redirect_uri   TEXT,
    created_at     TEXT NOT NULL,                    -- ISO 8601
    expires_at     TEXT NOT NULL,
    consumed_at    TEXT
);

CREATE INDEX idx_plane_a_oauth_states_cleanup
    ON plane_a_oauth_states (expires_at)
    WHERE consumed_at IS NULL;
