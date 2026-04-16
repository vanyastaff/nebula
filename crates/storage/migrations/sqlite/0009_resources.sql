-- 0009: Resources
-- Layer: Credentials & Resources
-- Spec: 16 (storage-schema), 25 (nebula-resource redesign)

CREATE TABLE resources (
    id             BLOB PRIMARY KEY,
    workspace_id   BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,
    config         TEXT NOT NULL,                    -- JSON
    created_at     TEXT NOT NULL,
    created_by     BLOB NOT NULL,
    version        INTEGER NOT NULL DEFAULT 0,
    deleted_at     TEXT
);

CREATE UNIQUE INDEX idx_resources_workspace_slug
    ON resources (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;
