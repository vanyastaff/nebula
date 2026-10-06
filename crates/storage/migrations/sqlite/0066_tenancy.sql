-- Tenancy: organizations, their workspaces, and the explicit grants that make
-- a principal a member of either.
--
-- Transition migration of the database standard (ADR-005): replaces the
-- port_* tenant tables and the dead legacy tenant tables. Instants are
-- INTEGER microseconds since the Unix epoch.

-- Dead legacy tables keyed to the legacy tenant tables, children first;
-- the live workflow, execution, trigger and resource tables are port_*.
DROP TABLE IF EXISTS pending_signals;
DROP TABLE IF EXISTS execution_nodes;
DROP TABLE IF EXISTS execution_journal;
DROP TABLE IF EXISTS execution_control_queue;
DROP TABLE IF EXISTS blobs;
DROP TABLE IF EXISTS executions;
DROP TABLE IF EXISTS trigger_events;
DROP TABLE IF EXISTS cron_fire_slots;
DROP TABLE IF EXISTS triggers;
DROP TABLE IF EXISTS workflow_versions;
DROP TABLE IF EXISTS workflows;
DROP TABLE IF EXISTS resources;

DROP TABLE IF EXISTS port_memberships;
DROP TABLE IF EXISTS port_workspaces;
DROP TABLE IF EXISTS port_orgs;
DROP TABLE IF EXISTS org_members;
DROP TABLE IF EXISTS workspace_members;
DROP TABLE IF EXISTS workspace_dispatch_state;
DROP TABLE IF EXISTS workspace_quota_usage;
DROP TABLE IF EXISTS org_quota_usage;
DROP TABLE IF EXISTS org_quotas;
DROP TABLE IF EXISTS slug_history;
DROP TABLE IF EXISTS workspaces;
DROP TABLE IF EXISTS orgs;

CREATE TABLE orgs (
    id            TEXT    NOT NULL,
    slug          TEXT    NOT NULL,
    display_name  TEXT    NOT NULL,
    plan          TEXT    NOT NULL,
    billing_email TEXT,
    settings      TEXT    NOT NULL DEFAULT '{}',
    created_by    TEXT    NOT NULL,
    created_at    INTEGER NOT NULL,
    version       INTEGER NOT NULL DEFAULT 0,
    deleted_at    INTEGER,
    CONSTRAINT pk_orgs PRIMARY KEY (id),
    CONSTRAINT ck_orgs__settings_json CHECK (json_valid(settings)),
    CONSTRAINT ck_orgs__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_orgs__active_slug ON orgs (slug) WHERE deleted_at IS NULL;

-- Workspace ids are unique across organizations; (org_id, id) is the key
-- grants reference so a grant always names its organization.
CREATE TABLE workspaces (
    org_id       TEXT    NOT NULL,
    id           TEXT    NOT NULL,
    slug         TEXT    NOT NULL,
    display_name TEXT    NOT NULL,
    description  TEXT,
    is_default   INTEGER NOT NULL DEFAULT 0,
    settings     TEXT    NOT NULL DEFAULT '{}',
    created_by   TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0,
    deleted_at   INTEGER,
    CONSTRAINT pk_workspaces PRIMARY KEY (id),
    CONSTRAINT uq_workspaces__org_id_id UNIQUE (org_id, id),
    CONSTRAINT fk_workspaces__orgs FOREIGN KEY (org_id) REFERENCES orgs (id),
    CONSTRAINT ck_workspaces__is_default CHECK (is_default IN (0, 1)),
    CONSTRAINT ck_workspaces__settings_json CHECK (json_valid(settings)),
    CONSTRAINT ck_workspaces__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_workspaces__active_slug ON workspaces (org_id, slug)
    WHERE deleted_at IS NULL;
-- An organization has at most one active default workspace.
CREATE UNIQUE INDEX uq_workspaces__active_default ON workspaces (org_id)
    WHERE is_default = 1 AND deleted_at IS NULL;

CREATE TABLE org_memberships (
    org_id         TEXT    NOT NULL,
    principal_kind TEXT    NOT NULL,
    principal_id   TEXT    NOT NULL,
    role           TEXT    NOT NULL,
    added_by       TEXT,
    added_at       INTEGER NOT NULL,
    CONSTRAINT pk_org_memberships PRIMARY KEY (org_id, principal_kind, principal_id),
    CONSTRAINT fk_org_memberships__orgs FOREIGN KEY (org_id) REFERENCES orgs (id),
    CONSTRAINT ck_org_memberships__principal_kind
        CHECK (principal_kind IN ('user', 'service_account')),
    CONSTRAINT ck_org_memberships__role
        CHECK (role IN ('OrgMember', 'OrgBilling', 'OrgAdmin', 'OrgOwner'))
);
CREATE INDEX ix_org_memberships__principal ON org_memberships (principal_kind, principal_id);

-- A workspace grant requires the principal's membership of the parent
-- organization; removing that membership removes its workspace grants.
CREATE TABLE workspace_memberships (
    org_id         TEXT    NOT NULL,
    workspace_id   TEXT    NOT NULL,
    principal_kind TEXT    NOT NULL,
    principal_id   TEXT    NOT NULL,
    role           TEXT    NOT NULL,
    added_by       TEXT,
    added_at       INTEGER NOT NULL,
    CONSTRAINT pk_workspace_memberships PRIMARY KEY (workspace_id, principal_kind, principal_id),
    CONSTRAINT fk_workspace_memberships__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id),
    CONSTRAINT fk_workspace_memberships__org_memberships
        FOREIGN KEY (org_id, principal_kind, principal_id)
        REFERENCES org_memberships (org_id, principal_kind, principal_id) ON DELETE CASCADE,
    CONSTRAINT ck_workspace_memberships__principal_kind
        CHECK (principal_kind IN ('user', 'service_account')),
    CONSTRAINT ck_workspace_memberships__role
        CHECK (role IN ('WorkspaceViewer', 'WorkspaceRunner', 'WorkspaceEditor', 'WorkspaceAdmin'))
);
CREATE INDEX ix_workspace_memberships__org_principal
    ON workspace_memberships (org_id, principal_kind, principal_id);
