-- Additive: capability-routed job-dispatch queue + trigger-dedup inbox.
-- Does not touch the production `execution_control_queue` table.
-- Reversible: DROP TABLE port_job_dispatch_queue, port_trigger_dedup_inbox.

CREATE TABLE IF NOT EXISTS port_job_dispatch_queue (
    id                  BYTEA PRIMARY KEY,
    execution_id        TEXT NOT NULL,
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    command             TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'Pending',
    payload             JSONB NOT NULL DEFAULT '{}',
    event_id            TEXT,
    target_flavor_sha   TEXT NOT NULL DEFAULT '',
    required_plugin_key TEXT NOT NULL,
    capability_tags     JSONB NOT NULL DEFAULT '[]',
    w3c_traceparent     TEXT,
    reclaim_count       INTEGER NOT NULL DEFAULT 0,
    processed_by        BYTEA,
    processed_at_ms     BIGINT,
    error_message       TEXT
);

CREATE INDEX IF NOT EXISTS idx_port_job_dispatch_queue_status_key
    ON port_job_dispatch_queue (status, required_plugin_key);

CREATE INDEX IF NOT EXISTS idx_port_job_dispatch_queue_tags
    ON port_job_dispatch_queue USING GIN (capability_tags);

CREATE TABLE IF NOT EXISTS port_trigger_dedup_inbox (
    trigger_id   TEXT NOT NULL,
    event_id     TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    org_id       TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (trigger_id, event_id)
);
