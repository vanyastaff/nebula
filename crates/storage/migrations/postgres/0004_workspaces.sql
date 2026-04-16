-- 0004: Workspaces
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 02 (tenancy-model)

CREATE TABLE workspaces (
    id             BYTEA PRIMARY KEY,                -- ws_ ULID
    org_id         BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    is_default     BOOLEAN NOT NULL DEFAULT FALSE,
    settings       JSONB NOT NULL DEFAULT '{}',
    version        BIGINT NOT NULL DEFAULT 0,        -- CAS
    deleted_at     TIMESTAMPTZ
);

-- Unique slug per org (active only)
CREATE UNIQUE INDEX idx_workspaces_org_slug
    ON workspaces (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;

-- Only one default workspace per org
CREATE UNIQUE INDEX idx_workspaces_org_default
    ON workspaces (org_id)
    WHERE is_default = TRUE AND deleted_at IS NULL;
