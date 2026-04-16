-- 0018: Webhook path fast-lookup
-- Insight: n8n stores webhook paths in a dedicated indexed column for O(1) routing.
-- Our triggers store webhook config in JSONB, but the dispatcher needs to resolve
-- incoming POST /hooks/{path} -> trigger_id without scanning all trigger configs.
--
-- Solution: extracted column + unique partial index on active webhook triggers.

ALTER TABLE triggers ADD COLUMN webhook_path TEXT;

-- Fast webhook dispatch: incoming request path -> trigger
CREATE UNIQUE INDEX idx_triggers_webhook_path
    ON triggers (webhook_path)
    WHERE webhook_path IS NOT NULL AND state = 'active' AND deleted_at IS NULL;
