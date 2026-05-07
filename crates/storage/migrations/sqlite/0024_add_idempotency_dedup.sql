-- M3.4 / ADR-0048 — durable idempotent-replay dedup store (SQLite parity).
--
-- `expires_at` stores milliseconds since the UNIX epoch (INTEGER) for the
-- same reason as `credential_refresh_claims` (migration 0022): integer
-- ordering is robust where lexicographic ISO-8601 ordering is fragile.
-- ADR-0009 (storage migrations parity) requires SQLite to track every PG
-- migration so the no-Docker dev path stays usable.

CREATE TABLE api_idempotency_dedup (
    cache_key   TEXT    PRIMARY KEY,
    status      INTEGER NOT NULL,                                -- u16 HTTP status
    headers     BLOB    NOT NULL,                                -- length-prefixed encoding
    body        BLOB    NOT NULL,
    fingerprint BLOB    NOT NULL,                                -- 32-byte SHA-256
    expires_at  INTEGER NOT NULL                                 -- millis since epoch (UTC)
);

CREATE INDEX idx_api_idempotency_dedup_expires_at
    ON api_idempotency_dedup (expires_at);
