-- 0008: Credentials
-- Layer: Credentials
-- Spec: 16 (storage-schema), 22 (credential-system)

CREATE TABLE credentials (
    id                  BYTEA PRIMARY KEY,           -- cred_ ULID
    org_id              BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id        BYTEA REFERENCES workspaces(id) ON DELETE CASCADE, -- NULL for org-level
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    kind                TEXT NOT NULL,               -- credential type key ('oauth2_google', 'api_key', etc.)
    scope               TEXT NOT NULL,               -- 'workspace' or 'org'
    encrypted_secret    BYTEA NOT NULL,              -- envelope-encrypted with org master key
    encryption_version  INT NOT NULL,                -- supports key rotation
    allowed_workspaces  BYTEA[],                     -- for org-level: list of allowed ws_ids
    metadata            JSONB,                       -- non-secret data (client_id, scopes, etc.)
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    last_rotated_at     TIMESTAMPTZ,
    last_used_at        TIMESTAMPTZ,
    version             BIGINT NOT NULL DEFAULT 0,   -- CAS
    deleted_at          TIMESTAMPTZ
);

-- Unique slug per workspace (workspace-scoped credentials)
CREATE UNIQUE INDEX idx_credentials_workspace_slug
    ON credentials (workspace_id, LOWER(slug))
    WHERE scope = 'workspace' AND deleted_at IS NULL;

-- Unique slug per org (org-scoped credentials)
CREATE UNIQUE INDEX idx_credentials_org_slug
    ON credentials (org_id, LOWER(slug))
    WHERE scope = 'org' AND deleted_at IS NULL;
