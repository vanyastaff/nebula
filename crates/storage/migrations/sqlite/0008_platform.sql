-- Platform persistence: HTTP response replay. Shared GCRA is PostgreSQL-only.

CREATE TABLE api_idempotency_dedup (
    cache_key TEXT NOT NULL,
    status INTEGER NOT NULL,
    headers BLOB NOT NULL,
    body BLOB NOT NULL,
    fingerprint BLOB NOT NULL,
    expires_at INTEGER NOT NULL,
    CONSTRAINT pk_api_idempotency_dedup PRIMARY KEY (cache_key),
    CONSTRAINT ck_api_idempotency_dedup__status CHECK (status BETWEEN 100 AND 999),
    CONSTRAINT ck_api_idempotency_dedup__fingerprint_length CHECK (typeof(fingerprint) = 'blob' AND length(fingerprint) = 32)
);
CREATE INDEX ix_api_idempotency_dedup__expires_at ON api_idempotency_dedup (expires_at);
