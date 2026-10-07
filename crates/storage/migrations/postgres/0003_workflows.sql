-- Workflows: the workflow row of a workspace and its immutable versions.
CREATE TABLE workflows (
    org_id       TEXT        NOT NULL,
    workspace_id TEXT        NOT NULL,
    id           TEXT        NOT NULL,
    slug         TEXT        NOT NULL,
    version      BIGINT      NOT NULL DEFAULT 0,
    deleted_at   TIMESTAMPTZ,
    CONSTRAINT pk_workflows PRIMARY KEY (org_id, workspace_id, id),
    CONSTRAINT fk_workflows__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_workflows__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_workflows__org_id_workspace_id_slug__live ON workflows (org_id, workspace_id, slug)
    WHERE deleted_at IS NULL;

-- `definition` is opaque to storage (the workflow compiler owns its shape).
-- `published` marks served versions (the highest number wins); `pinned`
-- excludes a version from automatic GC. An activated version records the
-- exact compiled plan and worker flavor it was published with — all three
-- identities or none.
CREATE TABLE workflow_versions (
    org_id                          TEXT    NOT NULL,
    workspace_id                    TEXT    NOT NULL,
    workflow_id                     TEXT    NOT NULL,
    number                          BIGINT  NOT NULL,
    published                       BOOLEAN NOT NULL DEFAULT FALSE,
    pinned                          BOOLEAN NOT NULL DEFAULT FALSE,
    definition                      JSONB   NOT NULL,
    activation_workflow_version_id  TEXT,
    activation_executable_plan_id   BYTEA,
    activation_worker_flavor_id     BYTEA,
    CONSTRAINT pk_workflow_versions PRIMARY KEY (org_id, workspace_id, workflow_id, number),
    CONSTRAINT uq_workflow_versions__activation_workflow_version_id
        UNIQUE (activation_workflow_version_id),
    CONSTRAINT fk_workflow_versions__workflows
        FOREIGN KEY (org_id, workspace_id, workflow_id)
        REFERENCES workflows (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_workflow_versions__number CHECK (number BETWEEN 0 AND 4294967295),
    CONSTRAINT ck_workflow_versions__activation_complete CHECK (
        (activation_workflow_version_id IS NULL
            AND activation_executable_plan_id IS NULL
            AND activation_worker_flavor_id IS NULL)
        OR (activation_workflow_version_id IS NOT NULL
            AND activation_executable_plan_id IS NOT NULL
            AND activation_worker_flavor_id IS NOT NULL)
    ),
    CONSTRAINT ck_workflow_versions__activation_executable_plan_id_length
        CHECK (octet_length(activation_executable_plan_id) = 32),
    CONSTRAINT ck_workflow_versions__activation_worker_flavor_id_length
        CHECK (octet_length(activation_worker_flavor_id) = 32)
);
CREATE INDEX ix_workflow_versions__workflow_id_number__published
    ON workflow_versions (org_id, workspace_id, workflow_id, number)
    WHERE published;
