-- The engine's resume seam: per-node raw outputs, per-node typed result
-- records and the workflow input of an execution. The execution aggregate owns
-- every row: it is keyed by the execution's tenant and purged with it. A write
-- replaces its slot (the latest write wins); the record's `schema_version` is
-- checked by the adapter on write and on read, so a newer writer's record
-- fails closed instead of being misread.
CREATE TABLE node_outputs (
    org_id         TEXT   NOT NULL,
    workspace_id   TEXT   NOT NULL,
    execution_id   TEXT   NOT NULL,
    node_key       TEXT   NOT NULL,
    kind_tag       TEXT   NOT NULL,
    payload        JSONB  NOT NULL,
    schema_version BIGINT NOT NULL,
    CONSTRAINT pk_node_outputs
        PRIMARY KEY (org_id, workspace_id, execution_id, node_key),
    CONSTRAINT fk_node_outputs__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_node_outputs__schema_version
        CHECK (schema_version BETWEEN 0 AND 4294967295)
);

CREATE TABLE node_results (
    org_id         TEXT   NOT NULL,
    workspace_id   TEXT   NOT NULL,
    execution_id   TEXT   NOT NULL,
    node_key       TEXT   NOT NULL,
    kind_tag       TEXT   NOT NULL,
    payload        JSONB  NOT NULL,
    schema_version BIGINT NOT NULL,
    CONSTRAINT pk_node_results
        PRIMARY KEY (org_id, workspace_id, execution_id, node_key),
    CONSTRAINT fk_node_results__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_node_results__schema_version
        CHECK (schema_version BETWEEN 0 AND 4294967295)
);

CREATE TABLE workflow_inputs (
    org_id         TEXT   NOT NULL,
    workspace_id   TEXT   NOT NULL,
    execution_id   TEXT   NOT NULL,
    kind_tag       TEXT   NOT NULL,
    payload        JSONB  NOT NULL,
    schema_version BIGINT NOT NULL,
    CONSTRAINT pk_workflow_inputs PRIMARY KEY (org_id, workspace_id, execution_id),
    CONSTRAINT fk_workflow_inputs__executions
        FOREIGN KEY (org_id, workspace_id, execution_id)
        REFERENCES executions (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_workflow_inputs__schema_version
        CHECK (schema_version BETWEEN 0 AND 4294967295)
);
