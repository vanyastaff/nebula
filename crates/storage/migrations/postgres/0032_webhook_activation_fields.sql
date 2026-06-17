-- ADR-0096 commit 1: extend port_webhook_activations with the fields
-- U-D1.4b needs (workflow_id, webhook_mode, token_hash).
--
-- All three columns are additive with safe defaults so existing rows are
-- not disturbed by the migration and the application can deploy without a
-- coordinated downtime.
--
-- workflow_id   : NULL = not yet wired to a specific workflow (filled by
--                 commit 2 / API rewire).
-- webhook_mode  : 'test' sentinel means "not yet promoted to production";
--                 the safe default ensures no existing activation silently
--                 becomes a live durable dispatch route.
-- token_hash    : 32-byte zero sentinel means "no capability token assigned
--                 yet".  The partial unique index below excludes the sentinel
--                 so the zero value is not required to be unique and all
--                 existing rows remain non-colliding.
--
-- Down (manual): DROP INDEX idx_port_webhook_activations_token_hash;
--                ALTER TABLE port_webhook_activations
--                  DROP COLUMN workflow_id,
--                  DROP COLUMN webhook_mode,
--                  DROP COLUMN token_hash;

ALTER TABLE port_webhook_activations
    ADD COLUMN workflow_id  TEXT,
    ADD COLUMN webhook_mode TEXT NOT NULL DEFAULT 'test'
        CHECK (webhook_mode IN ('test', 'prod')),
    ADD COLUMN token_hash   BYTEA NOT NULL
        DEFAULT decode(repeat('00', 32), 'hex');

-- Partial unique index on token_hash.  The sentinel (all-zeros) is excluded
-- so rows without an assigned token do not collide with each other.
-- A non-sentinel token_hash uniquely identifies at most one activation row,
-- enabling O(log n) resolve-by-token in commit 2.
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_webhook_activations_token_hash
    ON port_webhook_activations (token_hash)
    WHERE token_hash <> decode(repeat('00', 32), 'hex');
