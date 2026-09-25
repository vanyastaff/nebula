-- Cluster-wide rate limits (GCRA), shared by every worker process. One row
-- per limit key holds the theoretical arrival time of the key's next permit
-- and a mutation counter, both on the server clock (nanoseconds since the
-- Unix epoch, read from clock_timestamp()), plus the rate the key last
-- enforced, so callers that disagree on a key's rate get the stricter one
-- while the key is busy. Keys are opaque and namespaced by the caller (an
-- HMAC of tenant and provider account); no config or credential material is
-- stored. A row whose TAT has passed carries no state and is swept.
--
-- PostgreSQL-only: a SQLite deployment is one process, and there limits stay
-- in that process's memory.
CREATE TABLE port_rate_limits (
    limit_key TEXT PRIMARY KEY CHECK (octet_length(limit_key) BETWEEN 1 AND 512),
    tat_ns BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    emission_ns BIGINT NOT NULL CHECK (emission_ns > 0),
    burst INTEGER NOT NULL CHECK (burst > 0),
    -- End of the latest provider penalty (server clock, ns), so a caller
    -- that booked before it can find it after sleeping; the TAT a penalty
    -- sets is at least this, so the row outlives it.
    penalized_until_ns BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX port_rate_limits_tat ON port_rate_limits (tat_ns);

-- Reservations booked under a caller-chosen id, kept until their slot
-- arrives so a repeated request returns the original grant instead of
-- booking a second slot.
CREATE TABLE port_rate_limit_reservations (
    limit_key TEXT NOT NULL,
    reservation_id TEXT NOT NULL CHECK (octet_length(reservation_id) = 32),
    permits INTEGER NOT NULL CHECK (permits > 0),
    allow_at_ns BIGINT NOT NULL,
    end_tat_ns BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (limit_key, reservation_id)
);
CREATE INDEX port_rate_limit_reservations_allow_at ON port_rate_limit_reservations (allow_at_ns);
