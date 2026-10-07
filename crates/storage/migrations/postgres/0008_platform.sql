-- Platform persistence: durable HTTP response replay and shared GCRA limits.

CREATE TABLE api_idempotency_dedup (
    cache_key TEXT NOT NULL,
    status SMALLINT NOT NULL,
    headers BYTEA NOT NULL,
    body BYTEA NOT NULL,
    fingerprint BYTEA NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_api_idempotency_dedup PRIMARY KEY (cache_key),
    CONSTRAINT ck_api_idempotency_dedup__status CHECK (status BETWEEN 100 AND 999),
    CONSTRAINT ck_api_idempotency_dedup__fingerprint_length CHECK (octet_length(fingerprint) = 32)
);
CREATE INDEX ix_api_idempotency_dedup__expires_at ON api_idempotency_dedup (expires_at);

-- PostgreSQL-only: SQLite deployments use process-local limits.
-- GCRA coordinates retain integer nanoseconds and seq retains wrapping bits.
CREATE TABLE rate_limits (
    limit_key TEXT NOT NULL,
    tat_ns BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    emission_ns BIGINT NOT NULL,
    burst INTEGER NOT NULL,
    penalized_until_ns BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT pk_rate_limits PRIMARY KEY (limit_key),
    CONSTRAINT ck_rate_limits__limit_key_length CHECK (octet_length(limit_key) BETWEEN 1 AND 512),
    CONSTRAINT ck_rate_limits__tat_ns CHECK (tat_ns >= 0),
    CONSTRAINT ck_rate_limits__emission_ns CHECK (emission_ns > 0),
    CONSTRAINT ck_rate_limits__burst CHECK (burst > 0),
    CONSTRAINT ck_rate_limits__penalized_until_ns CHECK (penalized_until_ns >= 0)
);
CREATE INDEX ix_rate_limits__tat_ns ON rate_limits (tat_ns);

CREATE TABLE rate_limit_reservations (
    limit_key TEXT NOT NULL,
    reservation_id TEXT NOT NULL,
    permits INTEGER NOT NULL,
    allow_at_ns BIGINT NOT NULL,
    end_tat_ns BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    CONSTRAINT pk_rate_limit_reservations PRIMARY KEY (limit_key, reservation_id),
    CONSTRAINT fk_rate_limit_reservations__rate_limits FOREIGN KEY (limit_key)
        REFERENCES rate_limits (limit_key) ON DELETE CASCADE,
    CONSTRAINT ck_rate_limit_reservations__reservation_id_length CHECK (octet_length(reservation_id) = 32),
    CONSTRAINT ck_rate_limit_reservations__permits CHECK (permits > 0),
    CONSTRAINT ck_rate_limit_reservations__allow_at_ns CHECK (allow_at_ns >= 0),
    CONSTRAINT ck_rate_limit_reservations__end_tat_ns CHECK (end_tat_ns >= 0)
);
CREATE INDEX ix_rate_limit_reservations__allow_at_ns ON rate_limit_reservations (allow_at_ns);
