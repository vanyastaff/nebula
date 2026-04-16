-- 0005: Memberships and service accounts
-- Layer: Tenancy
-- Spec: 16 (storage-schema), 04 (rbac-sharing)

CREATE TABLE org_members (
    org_id             BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,
    principal_id       BLOB NOT NULL,
    role               TEXT NOT NULL,
    invited_at         TEXT NOT NULL,
    invited_by         BLOB,
    accepted_at        TEXT,
    PRIMARY KEY (org_id, principal_kind, principal_id)
);

CREATE INDEX idx_org_members_principal
    ON org_members (principal_kind, principal_id);

CREATE TABLE workspace_members (
    workspace_id       BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,
    principal_id       BLOB NOT NULL,
    role               TEXT NOT NULL,
    added_at           TEXT NOT NULL,
    added_by           BLOB NOT NULL,
    PRIMARY KEY (workspace_id, principal_kind, principal_id)
);

CREATE INDEX idx_workspace_members_principal
    ON workspace_members (principal_kind, principal_id);

CREATE TABLE service_accounts (
    id             BLOB PRIMARY KEY,
    org_id         BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TEXT NOT NULL,
    created_by     BLOB NOT NULL REFERENCES users(id),
    disabled_at    TEXT,
    deleted_at     TEXT
);

CREATE UNIQUE INDEX idx_sa_org_slug
    ON service_accounts (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;
