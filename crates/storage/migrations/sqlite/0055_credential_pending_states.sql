-- Durable, encrypted Plane-B interactive credential state.
-- The bearer token itself is never persisted; only its SHA-256 digest is stored.

CREATE TABLE credential_pending_states (
    token_digest    BLOB PRIMARY KEY NOT NULL CHECK (length(token_digest) = 32),
    credential_kind TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    session_id       TEXT NOT NULL,
    state_encrypted  BLOB NOT NULL,
    created_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    CHECK (expires_at > created_at)
);

CREATE INDEX idx_credential_pending_states_expiry
    ON credential_pending_states (expires_at);
