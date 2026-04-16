-- 0004: Workspaces
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 02 (tenancy-model)

CREATE TABLE workspaces (
    id             BLOB PRIMARY KEY,
    org_id         BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TEXT NOT NULL,
    created_by     BLOB NOT NULL,
    is_default     INTEGER NOT NULL DEFAULT 0,
    settings       TEXT NOT NULL DEFAULT '{}',
    version        INTEGER NOT NULL DEFAULT 0,
    deleted_at     TEXT
);

CREATE UNIQUE INDEX idx_workspaces_org_slug
    ON workspaces (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX idx_workspaces_org_default
    ON workspaces (org_id)
    WHERE is_default = 1 AND deleted_at IS NULL;
