-- Create the key-value storage table used by PostgresStorage.
CREATE TABLE IF NOT EXISTS storage_kv (
    key        TEXT        PRIMARY KEY,
    value      JSONB       NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
