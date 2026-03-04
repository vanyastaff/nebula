-- ============================================================
-- 029: Generic key-value storage (nebula-storage)
-- ============================================================

CREATE TABLE IF NOT EXISTS storage_kv (
    key         TEXT PRIMARY KEY,
    value       BYTEA NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_storage_kv_updated_at
    ON storage_kv (updated_at);

