-- ============================================================
-- 030: Align storage_kv value column to JSONB (nebula-storage)
-- ============================================================

-- If table does not exist yet, create it with JSONB contract.
CREATE TABLE IF NOT EXISTS storage_kv (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- If legacy BYTEA schema exists, convert to JSONB.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'storage_kv'
          AND column_name = 'value'
          AND udt_name = 'bytea'
    ) THEN
        ALTER TABLE storage_kv
            ALTER COLUMN value TYPE JSONB
            USING convert_from(value, 'UTF8')::jsonb;
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS idx_storage_kv_updated_at
    ON storage_kv (updated_at);
