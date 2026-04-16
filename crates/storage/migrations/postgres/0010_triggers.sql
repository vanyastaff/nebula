-- 0010: Triggers
-- Layer: Triggers
-- Spec: 16 (storage-schema), 11 (triggers)

-- ── Trigger definitions ────────────────────────────────────

CREATE TABLE triggers (
    id             BYTEA PRIMARY KEY,                -- trg_ ULID
    workspace_id   BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    workflow_id    BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,                    -- 'manual' / 'cron' / 'webhook' / 'event' / 'polling'
    config         JSONB NOT NULL,
    state          TEXT NOT NULL,                    -- 'active' / 'paused' / 'archived'
    run_as         BYTEA,                            -- ServiceAccountId; NULL = workspace default
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,        -- CAS
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_triggers_workspace_slug
    ON triggers (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_triggers_active
    ON triggers (workspace_id, state)
    WHERE state = 'active' AND deleted_at IS NULL;

-- ── Trigger events (inbox with dedup) ──────────────────────

CREATE TABLE trigger_events (
    id              BYTEA PRIMARY KEY,               -- evt_ ULID
    trigger_id      BYTEA NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    event_id        TEXT NOT NULL,                    -- author-configured or fallback hash
    received_at     TIMESTAMPTZ NOT NULL,
    claim_state     TEXT NOT NULL,                   -- 'pending' / 'claimed' / 'dispatched' / 'failed'
    claimed_by      BYTEA,                           -- dispatcher instance_id
    claimed_at      TIMESTAMPTZ,
    payload         JSONB NOT NULL,
    execution_id    BYTEA,                           -- set after execution created
    metadata        JSONB,

    UNIQUE (trigger_id, event_id)                    -- DEDUP ENFORCEMENT
);

CREATE INDEX idx_trigger_events_pending
    ON trigger_events (received_at)
    WHERE claim_state = 'pending';

CREATE INDEX idx_trigger_events_cleanup
    ON trigger_events (received_at)
    WHERE claim_state = 'dispatched';

-- ── Cron fire slots (leaderless coordination) ──────────────

CREATE TABLE cron_fire_slots (
    trigger_id      BYTEA NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    scheduled_for   TIMESTAMPTZ NOT NULL,
    claimed_by      BYTEA NOT NULL,                  -- dispatcher instance_id
    claimed_at      TIMESTAMPTZ NOT NULL,
    execution_id    BYTEA,                           -- populated after execution created
    PRIMARY KEY (trigger_id, scheduled_for)
);

CREATE INDEX idx_cron_fire_slots_cleanup
    ON cron_fire_slots (claimed_at);
