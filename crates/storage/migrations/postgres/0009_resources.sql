-- 0009: Resources
-- Layer: Credentials & Resources
-- Spec: 16 (storage-schema), 25 (nebula-resource redesign)

CREATE TABLE resources (
    id             BYTEA PRIMARY KEY,                -- res_ ULID
    workspace_id   BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,                    -- resource type key
    config         JSONB NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,        -- CAS
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_resources_workspace_slug
    ON resources (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;
