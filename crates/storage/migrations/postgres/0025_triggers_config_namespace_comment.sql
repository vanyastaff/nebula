-- 0025: triggers.config kind-namespaced JSONB contract (M3.3 / ADR-0049).
--
-- `triggers.config` is shared across every `triggers.kind`
-- (`manual` / `cron` / `webhook` / `event` / `polling`). Each kind
-- owns a top-level namespace key inside the JSON object so fields
-- cannot collide across kinds.
--
-- This migration attaches the canonical contract as `COMMENT ON
-- COLUMN` so DBA tooling, `\d+ triggers` in `psql`, and downstream
-- schema introspection see the shape without having to read Rust.
--
-- The contract is purely documentary — the application code in
-- `crates/storage/src/rows/webhook_activation.rs` (and sibling kind
-- decoders) is the enforcement seam. No data is rewritten.

COMMENT ON COLUMN triggers.config IS
'kind-namespaced JSONB. cron: { "schedule": text, "timezone": text }.'
' webhook: { "webhook_activation": WebhookActivationSpec }.'
' event:   { "event_types": [text, ...] }.'
' polling: { "interval_secs": integer, ... }.'
' Each kind owns its top-level key; sibling keys are preserved on'
' partial updates.';
