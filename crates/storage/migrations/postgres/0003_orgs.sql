-- 0003: Organizations
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 02 (tenancy-model)

CREATE TABLE orgs (
    id             BYTEA PRIMARY KEY,                -- org_ ULID
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,                   -- first user (no FK to preserve history)
    plan           TEXT NOT NULL,                     -- 'self_host' / 'free' / 'team' / 'business' / 'enterprise'
    billing_email  TEXT,
    settings       JSONB NOT NULL DEFAULT '{}',
    version        BIGINT NOT NULL DEFAULT 0,        -- CAS
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_orgs_slug_active
    ON orgs (LOWER(slug))
    WHERE deleted_at IS NULL;
