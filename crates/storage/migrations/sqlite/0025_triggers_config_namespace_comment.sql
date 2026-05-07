-- 0025: triggers.config kind-namespaced JSONB contract (M3.3 / ADR-0049).
--
-- SQLite mirror — kept for migration parity per ADR-0009 so the
-- no-Docker dev path stays in lockstep with PG. SQLite has no
-- `COMMENT ON COLUMN`, so this migration is intentionally a no-op;
-- the contract is enforced application-side by the row decoders in
-- `crates/storage/src/rows/webhook_activation.rs`.

SELECT 1;
