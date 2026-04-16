-- 0018: Webhook path fast-lookup

ALTER TABLE triggers ADD COLUMN webhook_path TEXT;

CREATE UNIQUE INDEX idx_triggers_webhook_path
    ON triggers (webhook_path)
    WHERE webhook_path IS NOT NULL AND state = 'active' AND deleted_at IS NULL;
