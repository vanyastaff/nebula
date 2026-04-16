-- 0010: Triggers
-- Layer: Triggers
-- Spec: 16 (storage-schema), 11 (triggers)

CREATE TABLE triggers (
    id             BLOB PRIMARY KEY,
    workspace_id   BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    workflow_id    BLOB NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,
    config         TEXT NOT NULL,                    -- JSON
    state          TEXT NOT NULL,
    run_as         BLOB,
    created_at     TEXT NOT NULL,
    created_by     BLOB NOT NULL,
    version        INTEGER NOT NULL DEFAULT 0,
    deleted_at     TEXT
);

CREATE UNIQUE INDEX idx_triggers_workspace_slug
    ON triggers (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_triggers_active
    ON triggers (workspace_id, state)
    WHERE state = 'active' AND deleted_at IS NULL;

CREATE TABLE trigger_events (
    id              BLOB PRIMARY KEY,
    trigger_id      BLOB NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    event_id        TEXT NOT NULL,
    received_at     TEXT NOT NULL,
    claim_state     TEXT NOT NULL,
    claimed_by      BLOB,
    claimed_at      TEXT,
    payload         TEXT NOT NULL,                   -- JSON
    execution_id    BLOB,
    metadata        TEXT,

    UNIQUE (trigger_id, event_id)
);

CREATE INDEX idx_trigger_events_pending
    ON trigger_events (received_at)
    WHERE claim_state = 'pending';

CREATE INDEX idx_trigger_events_cleanup
    ON trigger_events (received_at)
    WHERE claim_state = 'dispatched';

CREATE TABLE cron_fire_slots (
    trigger_id      BLOB NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    scheduled_for   TEXT NOT NULL,
    claimed_by      BLOB NOT NULL,
    claimed_at      TEXT NOT NULL,
    execution_id    BLOB,
    PRIMARY KEY (trigger_id, scheduled_for)
);

CREATE INDEX idx_cron_fire_slots_cleanup
    ON cron_fire_slots (claimed_at);
