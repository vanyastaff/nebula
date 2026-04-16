-- 0006: Workflows
-- Layer: Workflow
-- Spec: 16 (storage-schema), 13 (workflow-versioning)

CREATE TABLE workflows (
    id                  BLOB PRIMARY KEY,
    workspace_id        BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    description         TEXT,
    current_version_id  BLOB NOT NULL,
    state               TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    created_by          BLOB NOT NULL,
    updated_at          TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 0,
    deleted_at          TEXT
);

CREATE UNIQUE INDEX idx_workflows_workspace_slug
    ON workflows (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_workflows_state
    ON workflows (workspace_id, state)
    WHERE deleted_at IS NULL;
