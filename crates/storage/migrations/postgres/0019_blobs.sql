-- 0019: Binary/blob storage
-- Insight: n8n separates binary data (files, images, large payloads) from execution
-- JSONB. Windmill stores job logs in a separate `job_logs` table. Keeping multi-MB
-- blobs out of JSONB columns avoids index bloat and simplifies retention.
--
-- This table backs:
-- - execution_nodes.state_blob_ref (stateful action state > 1 MB, spec 14)
-- - Large node outputs (files, images from HTTP/S3 actions)
-- - Execution attachments (uploaded forms, generated reports)
--
-- Storage modes (configured per deployment):
--   'db'   — blob in `data` column (default, self-host simple)
--   'fs'   — blob on local filesystem, `data` NULL, `external_ref` = path
--   's3'   — blob in S3/MinIO, `data` NULL, `external_ref` = s3://bucket/key

CREATE TABLE blobs (
    id              BYTEA PRIMARY KEY,               -- ULID
    workspace_id    BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    execution_id    BYTEA REFERENCES executions(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,                   -- 'node_state' / 'node_output' / 'attachment' / 'log'
    content_type    TEXT,                            -- MIME type: 'application/json', 'image/png', etc.
    size_bytes      BIGINT NOT NULL,
    checksum        BYTEA,                           -- SHA-256 for integrity
    storage_mode    TEXT NOT NULL DEFAULT 'db',      -- 'db' / 'fs' / 's3'
    data            BYTEA,                           -- inline blob (NULL when fs/s3)
    external_ref    TEXT,                            -- fs path or s3:// URI (NULL when db)
    metadata        JSONB,                           -- kind-specific metadata
    created_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ                      -- NULL = retained with parent, set for temp blobs
);

-- Cleanup expired temp blobs
CREATE INDEX idx_blobs_expiry
    ON blobs (expires_at)
    WHERE expires_at IS NOT NULL;

-- Find blobs for an execution
CREATE INDEX idx_blobs_execution
    ON blobs (execution_id)
    WHERE execution_id IS NOT NULL;

-- Workspace-level storage accounting
CREATE INDEX idx_blobs_workspace
    ON blobs (workspace_id, created_at DESC);
