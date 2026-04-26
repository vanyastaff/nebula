-- Per ADR-0041 + sub-spec §3.4
-- One row per detected mid-refresh crash. Reclaim sweep inserts a row
-- when it finds an expired claim with sentinel=1. The threshold logic
-- (N=3 within 1h) lives in nebula-engine.
CREATE TABLE credential_sentinel_events (
    id              BIGSERIAL PRIMARY KEY,
    credential_id   TEXT NOT NULL,
    detected_at     TIMESTAMPTZ NOT NULL,
    crashed_holder  TEXT NOT NULL,
    generation      BIGINT NOT NULL
);

CREATE INDEX idx_sentinel_events_cred_time
    ON credential_sentinel_events(credential_id, detected_at);
