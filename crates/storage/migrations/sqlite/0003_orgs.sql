-- 0003: Organizations
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 02 (tenancy-model)

CREATE TABLE orgs (
    id             BLOB PRIMARY KEY,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    created_by     BLOB NOT NULL,
    plan           TEXT NOT NULL,
    billing_email  TEXT,
    settings       TEXT NOT NULL DEFAULT '{}',        -- JSON
    version        INTEGER NOT NULL DEFAULT 0,
    deleted_at     TEXT
);

CREATE UNIQUE INDEX idx_orgs_slug_active
    ON orgs (LOWER(slug))
    WHERE deleted_at IS NULL;
