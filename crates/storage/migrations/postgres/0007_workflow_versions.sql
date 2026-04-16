-- 0007: Workflow versions + FK cycle resolution
-- Layer: Workflow
-- Spec: 16 (storage-schema), 13 (workflow-versioning)

CREATE TABLE workflow_versions (
    id                    BYTEA PRIMARY KEY,          -- wfv_ ULID
    workflow_id           BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version_number        INT NOT NULL,
    definition            JSONB NOT NULL,
    schema_version        INT NOT NULL,
    state                 TEXT NOT NULL,              -- 'Draft' / 'Published' / 'Archived' / 'Deleted'
    created_at            TIMESTAMPTZ NOT NULL,
    created_by            BYTEA NOT NULL,
    description           TEXT,
    compiled_expressions  BYTEA,
    compiled_validation   BYTEA,
    pinned                BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (workflow_id, version_number)
);

-- Only one published version per workflow
CREATE UNIQUE INDEX idx_workflow_versions_published
    ON workflow_versions (workflow_id)
    WHERE state = 'Published';

CREATE INDEX idx_workflow_versions_by_workflow
    ON workflow_versions (workflow_id, version_number DESC);

-- Complete the FK cycle: workflows.current_version_id -> workflow_versions.id
ALTER TABLE workflows
    ADD CONSTRAINT fk_workflows_current_version
    FOREIGN KEY (current_version_id) REFERENCES workflow_versions(id);
