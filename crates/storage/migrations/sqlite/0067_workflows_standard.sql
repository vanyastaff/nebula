-- Workflows: the workflow row of a workspace and its immutable versions.
--
-- Transition migration of the database standard (ADR-005): replaces
-- port_workflows and port_workflow_versions. Instants are INTEGER
-- microseconds since the Unix epoch.

DROP TABLE IF EXISTS port_workflow_versions;
DROP TABLE IF EXISTS port_workflows;

CREATE TABLE workflows (
    org_id       TEXT    NOT NULL,
    workspace_id TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    slug         TEXT    NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0,
    deleted_at   INTEGER,
    CONSTRAINT pk_workflows PRIMARY KEY (org_id, workspace_id, id),
    CONSTRAINT fk_workflows__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id),
    CONSTRAINT ck_workflows__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_workflows__active_slug ON workflows (org_id, workspace_id, slug)
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
    number                          INTEGER NOT NULL,
    published                       INTEGER NOT NULL DEFAULT 0,
    pinned                          INTEGER NOT NULL DEFAULT 0,
    definition                      TEXT    NOT NULL,
    activation_workflow_version_id  TEXT,
    activation_executable_plan_id   BLOB,
    activation_worker_flavor_id     BLOB,
    CONSTRAINT pk_workflow_versions PRIMARY KEY (org_id, workspace_id, workflow_id, number),
    CONSTRAINT uq_workflow_versions__activation_workflow_version_id
        UNIQUE (activation_workflow_version_id),
    CONSTRAINT fk_workflow_versions__workflows
        FOREIGN KEY (org_id, workspace_id, workflow_id)
        REFERENCES workflows (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_workflow_versions__number CHECK (number BETWEEN 0 AND 4294967295),
    CONSTRAINT ck_workflow_versions__published CHECK (published IN (0, 1)),
    CONSTRAINT ck_workflow_versions__pinned CHECK (pinned IN (0, 1)),
    CONSTRAINT ck_workflow_versions__definition_json CHECK (json_valid(definition)),
    CONSTRAINT ck_workflow_versions__activation_complete CHECK (
        (activation_workflow_version_id IS NULL
            AND activation_executable_plan_id IS NULL
            AND activation_worker_flavor_id IS NULL)
        OR (activation_workflow_version_id IS NOT NULL
            AND activation_executable_plan_id IS NOT NULL
            AND activation_worker_flavor_id IS NOT NULL)
    ),
    CONSTRAINT ck_workflow_versions__activation_executable_plan_id_length
        CHECK (typeof(activation_executable_plan_id) IN ('null', 'blob')
            AND length(activation_executable_plan_id) = 32),
    CONSTRAINT ck_workflow_versions__activation_worker_flavor_id_length
        CHECK (typeof(activation_worker_flavor_id) IN ('null', 'blob')
            AND length(activation_worker_flavor_id) = 32)
);
CREATE INDEX ix_workflow_versions__published
    ON workflow_versions (org_id, workspace_id, workflow_id, number)
    WHERE published = 1;
