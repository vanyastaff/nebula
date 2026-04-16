-- 0019: Binary/blob storage

CREATE TABLE blobs (
    id              BLOB PRIMARY KEY,
    workspace_id    BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    execution_id    BLOB REFERENCES executions(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,
    content_type    TEXT,
    size_bytes      INTEGER NOT NULL,
    checksum        BLOB,
    storage_mode    TEXT NOT NULL DEFAULT 'db',
    data            BLOB,
    external_ref    TEXT,
    metadata        TEXT,                            -- JSON
    created_at      TEXT NOT NULL,
    expires_at      TEXT
);

CREATE INDEX idx_blobs_expiry
    ON blobs (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE INDEX idx_blobs_execution
    ON blobs (execution_id)
    WHERE execution_id IS NOT NULL;

CREATE INDEX idx_blobs_workspace
    ON blobs (workspace_id, created_at DESC);
