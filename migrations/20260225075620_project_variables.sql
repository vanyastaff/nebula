-- 027: Project Variables

CREATE TABLE project_variables (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key             VARCHAR(255) NOT NULL,
    value           TEXT NOT NULL,
    is_secret       BOOLEAN NOT NULL DEFAULT FALSE,
    description     TEXT,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, key)
);

CREATE INDEX idx_project_vars_project ON project_variables(project_id);
