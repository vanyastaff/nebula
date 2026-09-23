-- Admit immediately-expired pending state while preserving the released 0055 checksum.
-- SQLite cannot replace a CHECK constraint in place, so rebuild the relation.

DROP INDEX idx_credential_pending_states_expiry;

ALTER TABLE credential_pending_states
    RENAME TO credential_pending_states_0055;

CREATE TABLE credential_pending_states (
    token_digest    BLOB PRIMARY KEY NOT NULL CHECK (length(token_digest) = 32),
    credential_kind TEXT NOT NULL,
    owner_id         TEXT NOT NULL,
    session_id       TEXT NOT NULL,
    state_encrypted  BLOB NOT NULL,
    created_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    CHECK (expires_at >= created_at)
);

INSERT INTO credential_pending_states (
    token_digest,
    credential_kind,
    owner_id,
    session_id,
    state_encrypted,
    created_at,
    expires_at
)
SELECT
    token_digest,
    credential_kind,
    owner_id,
    session_id,
    state_encrypted,
    created_at,
    expires_at
FROM credential_pending_states_0055;

DROP TABLE credential_pending_states_0055;

CREATE INDEX idx_credential_pending_states_expiry
    ON credential_pending_states (expires_at);
