-- 0006: Workflows
-- Layer: Workflow
-- Spec: 16 (storage-schema), 13 (workflow-versioning)
--
-- Note: current_version_id FK is added in 0007 after workflow_versions exists.

CREATE TABLE workflows (
    id                  BYTEA PRIMARY KEY,           -- wf_ ULID
    workspace_id        BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    description         TEXT,
    current_version_id  BYTEA NOT NULL,              -- FK deferred to 0007
    state               TEXT NOT NULL,               -- 'Active' / 'Paused' / 'Archived'
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL,
    version             BIGINT NOT NULL DEFAULT 0,   -- CAS
    deleted_at          TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_workflows_workspace_slug
    ON workflows (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_workflows_state
    ON workflows (workspace_id, state)
    WHERE deleted_at IS NULL;
