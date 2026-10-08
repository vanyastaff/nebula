-- Tenancy: organizations, their workspaces, and the explicit grants that make
-- a principal a member of either.
CREATE TABLE orgs (
    id            TEXT        NOT NULL,
    slug          TEXT        NOT NULL,
    display_name  TEXT        NOT NULL,
    plan          TEXT        NOT NULL,
    billing_email TEXT,
    settings      JSONB       NOT NULL DEFAULT '{}',
    created_by    TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL,
    version       BIGINT      NOT NULL DEFAULT 0,
    deleted_at    TIMESTAMPTZ,
    CONSTRAINT pk_orgs PRIMARY KEY (id),
    CONSTRAINT ck_orgs__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_orgs__slug__live ON orgs (slug) WHERE deleted_at IS NULL;

-- Workspace ids are unique across organizations; (org_id, id) is the key
-- grants reference so a grant always names its organization.
CREATE TABLE workspaces (
    org_id       TEXT        NOT NULL,
    id           TEXT        NOT NULL,
    slug         TEXT        NOT NULL,
    display_name TEXT        NOT NULL,
    description  TEXT,
    is_default   BOOLEAN     NOT NULL DEFAULT FALSE,
    settings     JSONB       NOT NULL DEFAULT '{}',
    created_by   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL,
    version      BIGINT      NOT NULL DEFAULT 0,
    deleted_at   TIMESTAMPTZ,
    CONSTRAINT pk_workspaces PRIMARY KEY (id),
    CONSTRAINT uq_workspaces__org_id_id UNIQUE (org_id, id),
    CONSTRAINT fk_workspaces__orgs FOREIGN KEY (org_id) REFERENCES orgs (id) ON DELETE CASCADE,
    CONSTRAINT ck_workspaces__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_workspaces__org_id_slug__live ON workspaces (org_id, slug)
    WHERE deleted_at IS NULL;
-- An organization has at most one active default workspace.
CREATE UNIQUE INDEX uq_workspaces__org_id__live_default ON workspaces (org_id)
    WHERE is_default AND deleted_at IS NULL;

CREATE TABLE org_memberships (
    org_id         TEXT        NOT NULL,
    principal_kind TEXT        NOT NULL,
    principal_id   TEXT        NOT NULL,
    role           TEXT        NOT NULL,
    added_by       TEXT,
    added_at       TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_org_memberships PRIMARY KEY (org_id, principal_kind, principal_id),
    CONSTRAINT fk_org_memberships__orgs
        FOREIGN KEY (org_id) REFERENCES orgs (id) ON DELETE CASCADE,
    CONSTRAINT ck_org_memberships__principal_kind
        CHECK (principal_kind IN ('user', 'service_account')),
    CONSTRAINT ck_org_memberships__role
        CHECK (role IN ('OrgMember', 'OrgBilling', 'OrgAdmin', 'OrgOwner'))
);
CREATE INDEX ix_org_memberships__principal_kind_principal_id ON org_memberships (principal_kind, principal_id);

-- A workspace grant requires the principal's membership of the parent
-- organization; removing that membership removes its workspace grants.
CREATE TABLE workspace_memberships (
    org_id         TEXT        NOT NULL,
    workspace_id   TEXT        NOT NULL,
    principal_kind TEXT        NOT NULL,
    principal_id   TEXT        NOT NULL,
    role           TEXT        NOT NULL,
    added_by       TEXT,
    added_at       TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_workspace_memberships PRIMARY KEY (workspace_id, principal_kind, principal_id),
    CONSTRAINT fk_workspace_memberships__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_workspace_memberships__org_memberships
        FOREIGN KEY (org_id, principal_kind, principal_id)
        REFERENCES org_memberships (org_id, principal_kind, principal_id) ON DELETE CASCADE,
    CONSTRAINT ck_workspace_memberships__principal_kind
        CHECK (principal_kind IN ('user', 'service_account')),
    CONSTRAINT ck_workspace_memberships__role
        CHECK (role IN ('WorkspaceViewer', 'WorkspaceRunner', 'WorkspaceEditor', 'WorkspaceAdmin'))
);
CREATE INDEX ix_workspace_memberships__org_id_principal_kind_principal_id
    ON workspace_memberships (org_id, principal_kind, principal_id);
