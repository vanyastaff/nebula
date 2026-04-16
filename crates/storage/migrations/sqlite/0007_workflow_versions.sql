-- 0007: Workflow versions
-- Layer: Workflow
-- Spec: 16 (storage-schema), 13 (workflow-versioning)
--
-- Note: SQLite does not support ALTER TABLE ADD CONSTRAINT for FK.
-- The FK from workflows.current_version_id is enforced at the application level.

CREATE TABLE workflow_versions (
    id                    BLOB PRIMARY KEY,
    workflow_id           BLOB NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version_number        INTEGER NOT NULL,
    definition            TEXT NOT NULL,              -- JSON
    schema_version        INTEGER NOT NULL,
    state                 TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    created_by            BLOB NOT NULL,
    description           TEXT,
    compiled_expressions  BLOB,
    compiled_validation   BLOB,
    pinned                INTEGER NOT NULL DEFAULT 0,
    UNIQUE (workflow_id, version_number)
);

CREATE UNIQUE INDEX idx_workflow_versions_published
    ON workflow_versions (workflow_id)
    WHERE state = 'Published';

CREATE INDEX idx_workflow_versions_by_workflow
    ON workflow_versions (workflow_id, version_number DESC);
