-- ============================================================
-- 009: Enterprise RBAC — Roles, Projects, Folders, Teams
-- ============================================================
--
-- Permission resolution chain (highest -> lowest precedence):
--   1. instance_admin / org_owner  -> full access
--   2. org_role (on org_members)   -> org-wide floor
--   3. project_member role          -> resources within project
--   4. team project role            -> inherited via team membership
--   5. shared_workflow / shared_credential -> individual overrides
--   6. explicit deny (acl_entries)  -> always wins
-- ============================================================

-- ============================================================
-- PERMISSION SCOPES (catalogue, not enforced by DB)
-- ============================================================
-- workflow:create, workflow:read, workflow:update, workflow:delete,
--   workflow:list, workflow:execute, workflow:publish,
--   workflow:move, workflow:share, workflow:debug
-- credential:create, credential:read, credential:update, credential:delete,
--   credential:list, credential:move, credential:share, credential:use
-- execution:read, execution:list, execution:cancel, execution:delete, execution:retry
-- project:create, project:read, project:update, project:delete, project:list
-- folder:create, folder:read, folder:update, folder:delete, folder:list, folder:move
-- variable:create, variable:read, variable:update, variable:delete, variable:list
-- member:invite, member:remove, member:update_role
-- tag:create, tag:read, tag:update, tag:delete          (instance-scoped, no project RBAC)
-- audit:read
-- resource:create, resource:read, resource:update, resource:delete, resource:list

-- ============================================================
-- CUSTOM ROLES
-- ============================================================

CREATE TABLE roles (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id     UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name                VARCHAR(128) NOT NULL,
    description         TEXT,
    is_builtin          BOOLEAN NOT NULL DEFAULT FALSE,   -- built-in cannot be deleted
    -- If non-null, inherits all scopes of this built-in role and adds extras
    inherits_role       VARCHAR(64),                      -- 'project_admin'|'project_editor'|'project_viewer'
    -- Explicit scope grants, stored as sorted array
    -- e.g. ['workflow:create','workflow:read','credential:use']
    scopes              TEXT[] NOT NULL DEFAULT '{}',
    -- Explicit scope denials (overrides everything, use sparingly)
    denied_scopes       TEXT[] NOT NULL DEFAULT '{}',
    created_by          UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, name)
);

CREATE INDEX idx_roles_org ON roles(organization_id);

-- Seed built-in roles per org is done at application boot via upsert,
-- or via a trigger. Schema only defines the structure.

-- Built-in project roles (reference values, enforced in app logic):
-- project_admin   -> all scopes on project resources
-- project_editor  -> create/read/update/execute on workflows+credentials, no delete/move
-- project_viewer  -> read/list only, no execute
-- project_runner  -> execute only (CI/CD service accounts)

-- ============================================================
-- PROJECTS  (primary RBAC boundary within a tenant)
-- ============================================================
-- A Project groups workflows + credentials under shared permissions.
-- Every user gets an auto-created personal project (type = 'personal').
-- Team projects are type = 'team'.
-- ============================================================

CREATE TYPE project_type AS ENUM ('personal', 'team');

CREATE TABLE projects (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    type            project_type NOT NULL DEFAULT 'team',

    -- Personal project: tied to exactly one user
    owner_user_id   UUID REFERENCES users(id) ON DELETE CASCADE,

    icon            VARCHAR(64),                           -- emoji or icon name
    color           VARCHAR(16),                           -- hex color for UI
    settings        JSONB NOT NULL DEFAULT '{}',
    is_archived     BOOLEAN NOT NULL DEFAULT FALSE,

    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_projects_tenant ON projects(tenant_id);
CREATE INDEX idx_projects_type ON projects(type);
-- Only one personal project per user per tenant
CREATE UNIQUE INDEX idx_projects_personal_user
    ON projects(tenant_id, owner_user_id)
    WHERE type = 'personal';

-- ============================================================
-- FOLDERS  (hierarchy within projects, like n8n folders)
-- ============================================================

CREATE TABLE folders (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id  UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    parent_id   UUID REFERENCES folders(id) ON DELETE CASCADE,
    name        VARCHAR(255) NOT NULL,
    path        TEXT NOT NULL,                              -- materialized path: '/eng/data-pipelines/etl'
    position    INTEGER NOT NULL DEFAULT 0,                 -- sort order within parent
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, parent_id, name)
);

CREATE INDEX idx_folders_project ON folders(project_id);
CREATE INDEX idx_folders_parent ON folders(parent_id);
CREATE INDEX idx_folders_path ON folders USING gin(path gin_trgm_ops);

-- ============================================================
-- PROJECT MEMBERS  (user <-> project with role)
-- ============================================================

CREATE TABLE project_members (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id         UUID REFERENCES users(id) ON DELETE CASCADE,
    -- NULL user_id + non-null team_id = team as member (expands to team's users)
    team_id         UUID,                                   -- FK added after teams table
    -- Either built-in role name OR custom role id (not both)
    builtin_role    VARCHAR(64),                            -- 'project_admin'|'project_editor'|'project_viewer'|'project_runner'
    custom_role_id  UUID REFERENCES roles(id) ON DELETE SET NULL,

    invited_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT chk_member_target CHECK (
        (user_id IS NOT NULL AND team_id IS NULL) OR
        (user_id IS NULL AND team_id IS NOT NULL)
    ),
    CONSTRAINT chk_role_set CHECK (
        (builtin_role IS NOT NULL) != (custom_role_id IS NOT NULL)
    ),
    UNIQUE NULLS NOT DISTINCT (project_id, user_id),
    UNIQUE NULLS NOT DISTINCT (project_id, team_id)
);

CREATE INDEX idx_project_members_project ON project_members(project_id);
CREATE INDEX idx_project_members_user ON project_members(user_id) WHERE user_id IS NOT NULL;
CREATE INDEX idx_project_members_team ON project_members(team_id) WHERE team_id IS NOT NULL;

-- ============================================================
-- TEAMS  (user groups within an organization)
-- ============================================================

CREATE TABLE teams (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            VARCHAR(128) NOT NULL,
    description     TEXT,
    icon            VARCHAR(64),
    color           VARCHAR(16),
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, name)
);

CREATE INDEX idx_teams_org ON teams(organization_id);

-- Team membership
CREATE TABLE team_members (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    team_id     UUID NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    is_manager  BOOLEAN NOT NULL DEFAULT FALSE,            -- team managers can add/remove members
    added_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (team_id, user_id)
);

CREATE INDEX idx_team_members_team ON team_members(team_id);
CREATE INDEX idx_team_members_user ON team_members(user_id);

-- Backfill team FK into project_members
ALTER TABLE project_members
    ADD CONSTRAINT fk_pm_team
    FOREIGN KEY (team_id) REFERENCES teams(id) ON DELETE CASCADE;
