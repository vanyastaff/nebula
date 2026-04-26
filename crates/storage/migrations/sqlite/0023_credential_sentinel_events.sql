-- Per ADR-0041 + sub-spec §3.4
-- One row per detected mid-refresh crash. Reclaim sweep inserts a row
-- when it finds an expired claim with sentinel=1. The threshold logic
-- (N=3 within 1h) lives in nebula-engine.
CREATE TABLE credential_sentinel_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    credential_id     TEXT    NOT NULL,
    detected_at       TEXT    NOT NULL,                          -- ISO-8601
    crashed_holder    TEXT    NOT NULL,                          -- replica id
    generation        INTEGER NOT NULL                            -- claim row's generation at crash
);

CREATE INDEX idx_sentinel_events_cred_time
    ON credential_sentinel_events(credential_id, detected_at);
