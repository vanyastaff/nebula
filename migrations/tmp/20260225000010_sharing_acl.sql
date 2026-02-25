-- ============================================================
-- 010: Sharing & Access Control Lists
-- ============================================================

-- ============================================================
-- SHARED WORKFLOWS  (resource-level ACL on workflows)
-- ============================================================
-- A workflow always belongs to exactly ONE home project (home=true).
-- It can be additionally shared with other projects (home=false).
-- ============================================================

CREATE TABLE shared_workflows (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- 'owner': full control (home project)
    -- 'editor': can edit and execute
    -- 'viewer': read-only
    -- 'executor': execute-only (for CI/CD pipelines consuming others' workflows)
    role            VARCHAR(32) NOT NULL DEFAULT 'editor',
    is_home         BOOLEAN NOT NULL DEFAULT FALSE,        -- exactly one row per workflow must be home=true
    shared_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workflow_id, project_id)
);

CREATE INDEX idx_shared_wf_workflow ON shared_workflows(workflow_id);
CREATE INDEX idx_shared_wf_project ON shared_workflows(project_id);
-- Enforce single home project per workflow
CREATE UNIQUE INDEX idx_shared_wf_home
    ON shared_workflows(workflow_id)
    WHERE is_home = TRUE;

-- Workflow <-> Folder assignment
ALTER TABLE workflows
    ADD COLUMN folder_id UUID REFERENCES folders(id) ON DELETE SET NULL;
CREATE INDEX idx_workflows_folder ON workflows(folder_id);

-- ============================================================
-- SHARED CREDENTIALS  (resource-level ACL on credentials)
-- ============================================================
-- Credentials are scoped to a project by default.
-- They can be shared with additional projects or individual users.
-- Consumers can USE (execute) but never SEE the raw secret.
-- ============================================================

CREATE TYPE credential_share_target AS ENUM ('project', 'user');

CREATE TABLE shared_credentials (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    credential_id   UUID NOT NULL REFERENCES credentials(id) ON DELETE CASCADE,

    -- Target: project OR user (not both)
    target_type     credential_share_target NOT NULL,
    target_project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    target_user_id    UUID REFERENCES users(id) ON DELETE CASCADE,

    -- 'owner': can edit/delete/reshare
    -- 'editor': can edit (update encrypted data)
    -- 'user': can USE in workflow executions only (no read of raw data)
    role            VARCHAR(32) NOT NULL DEFAULT 'user',
    is_home         BOOLEAN NOT NULL DEFAULT FALSE,        -- home project owns the credential

    shared_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT chk_cred_target CHECK (
        (target_type = 'project' AND target_project_id IS NOT NULL AND target_user_id IS NULL) OR
        (target_type = 'user' AND target_user_id IS NOT NULL AND target_project_id IS NULL)
    ),
    UNIQUE NULLS NOT DISTINCT (credential_id, target_project_id),
    UNIQUE NULLS NOT DISTINCT (credential_id, target_user_id)
);

CREATE INDEX idx_shared_cred_credential ON shared_credentials(credential_id);
CREATE INDEX idx_shared_cred_project ON shared_credentials(target_project_id) WHERE target_project_id IS NOT NULL;
CREATE INDEX idx_shared_cred_user ON shared_credentials(target_user_id) WHERE target_user_id IS NOT NULL;

-- Home credential per project
CREATE UNIQUE INDEX idx_shared_cred_home
    ON shared_credentials(credential_id)
    WHERE is_home = TRUE;

-- Add home project FK to credentials table
ALTER TABLE credentials ADD COLUMN project_id UUID REFERENCES projects(id) ON DELETE SET NULL;
CREATE INDEX idx_credentials_project ON credentials(project_id);

-- ============================================================
-- OBJECT-LEVEL ACL (explicit allow/deny for individual resources)
-- ============================================================
-- Used for fine-grained overrides beyond project membership.
-- E.g.: Deny user X from executing workflow Y even though they're a project editor.
-- ============================================================

CREATE TYPE acl_resource_type AS ENUM ('workflow', 'credential', 'project', 'folder', 'execution');
CREATE TYPE acl_principal_type AS ENUM ('user', 'team', 'service_account');
CREATE TYPE acl_effect AS ENUM ('allow', 'deny');

CREATE TABLE acl_entries (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    -- Resource
    resource_type   acl_resource_type NOT NULL,
    resource_id     UUID NOT NULL,
    -- Principal
    principal_type  acl_principal_type NOT NULL,
    principal_id    UUID NOT NULL,
    -- Permission
    scope           VARCHAR(128) NOT NULL,                 -- 'workflow:execute', 'credential:use', etc.
    effect          acl_effect NOT NULL DEFAULT 'allow',
    -- Metadata
    reason          TEXT,                                  -- audit reason for the ACL entry
    expires_at      TIMESTAMPTZ,                           -- temporary ACL (e.g., contractor access)
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (resource_type, resource_id, principal_type, principal_id, scope)
);

CREATE INDEX idx_acl_resource ON acl_entries(resource_type, resource_id);
CREATE INDEX idx_acl_principal ON acl_entries(principal_type, principal_id);
CREATE INDEX idx_acl_expires ON acl_entries(expires_at) WHERE expires_at IS NOT NULL;
