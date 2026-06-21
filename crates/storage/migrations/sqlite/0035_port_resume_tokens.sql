-- ADR-0099 W-S3c: mint-on-park resume token store.
--
-- Tokens are minted by the engine in the SAME transaction as the
-- Waiting-state snapshot (TransitionBatch.resume_tokens).  The
-- token_hash is the SHA-256 of the 32-byte CSPRNG plaintext — stored
-- as raw bytes (BLOB), NOT as hex text, so SQLite BLOB comparison is
-- exact and cannot be confused by case-folding collations.
--
-- token_hash CHECK: SQLite length() on a BLOB returns byte length.
-- UNIQUE(execution_id, node_key): ON CONFLICT DO NOTHING prevents a
-- crash re-drive from minting a second live token for the same parked
-- node.
-- CASCADE delete: when port_executions row is deleted the token is gone.
-- Index on (workspace_id, org_id, execution_id): revoke_on_terminal sweep.

CREATE TABLE IF NOT EXISTS port_resume_tokens (
    token_hash      BLOB    NOT NULL PRIMARY KEY
                            CHECK (length(token_hash) = 32),
    workspace_id    TEXT    NOT NULL,
    org_id          TEXT    NOT NULL,
    execution_id    TEXT    NOT NULL,
    node_key        TEXT    NOT NULL,
    wait_kind       TEXT    NOT NULL,
    callback_label  TEXT    NOT NULL,
    created_at      TEXT    NOT NULL,
    expires_at      TEXT,
    UNIQUE (execution_id, node_key),
    FOREIGN KEY (execution_id)
        REFERENCES port_executions (id)
        ON DELETE CASCADE,
    CHECK (wait_kind IN ('webhook', 'approval'))
);

CREATE INDEX IF NOT EXISTS idx_port_resume_tokens_execution
    ON port_resume_tokens (workspace_id, org_id, execution_id);
