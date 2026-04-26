-- Per ADR-0041 + sub-spec §3.3
-- Holds in-flight refresh claims for cross-replica coordination.
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT    NOT NULL PRIMARY KEY,
    claim_id          TEXT    NOT NULL,                        -- UUID
    generation        INTEGER NOT NULL,                          -- bumped on each CAS
    holder_replica_id TEXT    NOT NULL,
    acquired_at       TEXT    NOT NULL,                          -- ISO-8601
    expires_at        TEXT    NOT NULL,
    sentinel          INTEGER NOT NULL DEFAULT 0,                -- 0=normal, 1=refresh_in_flight
    CHECK (sentinel IN (0, 1))
);

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);
