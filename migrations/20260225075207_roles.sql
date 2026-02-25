-- 017: Roles (Custom RBAC)

-- ============================================================
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
