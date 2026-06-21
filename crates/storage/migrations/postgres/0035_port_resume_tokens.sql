-- ADR-0099 W-S3c: mint-on-park resume token store.
--
-- Tokens are minted by the engine in the SAME transaction as the
-- Waiting-state snapshot (TransitionBatch.resume_tokens).  The
-- token_hash is the SHA-256 of the 32-byte CSPRNG plaintext — stored
-- as raw BYTEA (not hex text) so there is no collation or encoding
-- ambiguity on exact-match lookups.
--
-- token_hash CHECK: octet_length() on BYTEA is the byte count.
-- UNIQUE(execution_id, node_key): ON CONFLICT DO NOTHING prevents a
-- crash re-drive from minting a second live token for the same parked
-- node.
-- CASCADE delete: when port_executions row is deleted the token is gone.
-- Index on (workspace_id, org_id, execution_id): revoke_on_terminal sweep.

CREATE TABLE IF NOT EXISTS port_resume_tokens (
    token_hash      BYTEA       NOT NULL PRIMARY KEY
                                CHECK (octet_length(token_hash) = 32),
    workspace_id    TEXT        NOT NULL,
    org_id          TEXT        NOT NULL,
    execution_id    TEXT        NOT NULL,
    node_key        TEXT        NOT NULL,
    wait_kind       TEXT        NOT NULL,
    callback_label  TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ,
    UNIQUE (execution_id, node_key),
    -- Single-column FK: port_executions PK is `id TEXT` alone; there is no
    -- UNIQUE constraint on (workspace_id, org_id, id), so a composite FK
    -- would fail with SQLSTATE 42830.  workspace_id / org_id are kept as
    -- data columns (used by the revoke_on_terminal sweep index below).
    FOREIGN KEY (execution_id)
        REFERENCES port_executions (id)
        ON DELETE CASCADE,
    CHECK (wait_kind IN ('webhook', 'approval'))
);

CREATE INDEX IF NOT EXISTS idx_port_resume_tokens_execution
    ON port_resume_tokens (workspace_id, org_id, execution_id);
