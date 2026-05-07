-- M3.4 / ADR-0048 — durable idempotent-replay dedup store.
--
-- One row per `(method, path, Idempotency-Key, identity-fingerprint,
-- body-fingerprint)` tuple. The middleware consults this table on every
-- POST that carries an `Idempotency-Key` header; first writer wins via
-- `INSERT ... ON CONFLICT (cache_key) DO NOTHING`. Rows expire on
-- `expires_at` (sweep task in the API composition root drives the
-- `evict_expired` call).

CREATE TABLE IF NOT EXISTS api_idempotency_dedup (
    cache_key   TEXT PRIMARY KEY,
    status      SMALLINT NOT NULL,
    headers     BYTEA NOT NULL,
    body        BYTEA NOT NULL,
    fingerprint BYTEA NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS api_idempotency_dedup_expires_at_idx
    ON api_idempotency_dedup (expires_at);
