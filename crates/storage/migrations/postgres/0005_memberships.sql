-- 0005: Memberships and service accounts
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 04 (rbac-sharing)

-- ── Org members ────────────────────────────────────────────

CREATE TABLE org_members (
    org_id             BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,                 -- 'user' / 'service_account'
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,                 -- 'OrgOwner' / 'OrgAdmin' / 'OrgMember' / 'OrgBilling'
    invited_at         TIMESTAMPTZ NOT NULL,
    invited_by         BYTEA,
    accepted_at        TIMESTAMPTZ,
    PRIMARY KEY (org_id, principal_kind, principal_id)
);

CREATE INDEX idx_org_members_principal
    ON org_members (principal_kind, principal_id);

-- ── Workspace members ──────────────────────────────────────

CREATE TABLE workspace_members (
    workspace_id       BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,                 -- 'WorkspaceAdmin' / 'Editor' / 'Runner' / 'Viewer'
    added_at           TIMESTAMPTZ NOT NULL,
    added_by           BYTEA NOT NULL,
    PRIMARY KEY (workspace_id, principal_kind, principal_id)
);

CREATE INDEX idx_workspace_members_principal
    ON workspace_members (principal_kind, principal_id);

-- ── Service accounts ───────────────────────────────────────

CREATE TABLE service_accounts (
    id             BYTEA PRIMARY KEY,                -- svc_ ULID
    org_id         BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL REFERENCES users(id),
    disabled_at    TIMESTAMPTZ,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_sa_org_slug
    ON service_accounts (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;
