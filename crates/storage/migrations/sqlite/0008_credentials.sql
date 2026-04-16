-- 0008: Credentials
-- Layer: Credentials
-- Spec: 16 (storage-schema), 22 (credential-system)

CREATE TABLE credentials (
    id                  BLOB PRIMARY KEY,
    org_id              BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id        BLOB REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    kind                TEXT NOT NULL,
    scope               TEXT NOT NULL,
    encrypted_secret    BLOB NOT NULL,
    encryption_version  INTEGER NOT NULL,
    allowed_workspaces  TEXT,                         -- JSON array of workspace IDs
    metadata            TEXT,                         -- JSON
    created_at          TEXT NOT NULL,
    created_by          BLOB NOT NULL,
    last_rotated_at     TEXT,
    last_used_at        TEXT,
    version             INTEGER NOT NULL DEFAULT 0,
    deleted_at          TEXT
);

CREATE UNIQUE INDEX idx_credentials_workspace_slug
    ON credentials (workspace_id, LOWER(slug))
    WHERE scope = 'workspace' AND deleted_at IS NULL;

CREATE UNIQUE INDEX idx_credentials_org_slug
    ON credentials (org_id, LOWER(slug))
    WHERE scope = 'org' AND deleted_at IS NULL;
