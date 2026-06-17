-- ADR-0096 commit 1: extend port_webhook_activations with the fields
-- U-D1.4b needs (workflow_id, webhook_mode, token_hash).
--
-- SQLite requires one ALTER TABLE … ADD COLUMN statement per column
-- (multi-column ALTER TABLE is not supported).
--
-- All three additions use safe defaults so existing rows are not disturbed:
--   workflow_id   NULL   → not yet wired to a specific workflow
--   webhook_mode  'test' → not yet promoted to production (safe default —
--                          no existing activation silently becomes a live
--                          durable dispatch route)
--   token_hash    X'00…' → zero sentinel, no capability token assigned yet
--
-- The partial unique index on token_hash excludes the zero sentinel so
-- existing rows (all sentinel) do not collide.
--
-- Down (manual): DROP INDEX idx_port_webhook_activations_token_hash;
--                (SQLite has no DROP COLUMN before 3.35; migrate down by
--                 recreating the table without the columns.)

ALTER TABLE port_webhook_activations
    ADD COLUMN workflow_id TEXT;

ALTER TABLE port_webhook_activations
    ADD COLUMN webhook_mode TEXT NOT NULL DEFAULT 'test'
        CHECK (webhook_mode IN ('test', 'prod'));

ALTER TABLE port_webhook_activations
    ADD COLUMN token_hash BLOB NOT NULL
        DEFAULT X'0000000000000000000000000000000000000000000000000000000000000000'
        CHECK (length(token_hash) = 32);

-- Partial unique index: the sentinel (all-zeros 32 bytes) is excluded so
-- rows without an assigned token do not collide with each other.
-- A non-sentinel token_hash uniquely identifies at most one activation row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_port_webhook_activations_token_hash
    ON port_webhook_activations (token_hash)
    WHERE token_hash <> X'0000000000000000000000000000000000000000000000000000000000000000';
