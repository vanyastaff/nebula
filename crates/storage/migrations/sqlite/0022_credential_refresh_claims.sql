-- Per ADR-0041 + sub-spec §3.3
-- Holds in-flight refresh claims for cross-replica coordination.
--
-- Timestamp columns store milliseconds since the UNIX epoch (INTEGER) rather
-- than ISO-8601 strings. Lexicographic comparison of `to_rfc3339()` is unsafe
-- across chrono versions and mixed inserts (the conditional fractional-second
-- suffix and `Z` vs `+00:00` zone forms break ordering); INTEGER is naturally
-- comparable for the `expires_at < now` predicate used by `try_claim` and
-- `reclaim_stuck`. Postgres mirrors this with native `TIMESTAMPTZ`.
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT    NOT NULL PRIMARY KEY,
    claim_id          TEXT    NOT NULL,                        -- UUID
    generation        INTEGER NOT NULL,                          -- bumped on each CAS
    holder_replica_id TEXT    NOT NULL,
    acquired_at       INTEGER NOT NULL,                          -- millis since epoch (UTC)
    expires_at        INTEGER NOT NULL,                          -- millis since epoch (UTC)
    sentinel          INTEGER NOT NULL DEFAULT 0,                -- 0=normal, 1=refresh_in_flight
    CHECK (sentinel IN (0, 1))
);

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);
