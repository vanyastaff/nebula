-- Per ADR-0041 + sub-spec §3.3
-- Holds in-flight refresh claims for cross-replica coordination.
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT NOT NULL PRIMARY KEY,
    claim_id          UUID NOT NULL,
    generation        BIGINT NOT NULL,
    holder_replica_id TEXT NOT NULL,
    acquired_at       TIMESTAMPTZ NOT NULL,
    expires_at        TIMESTAMPTZ NOT NULL,
    sentinel          SMALLINT NOT NULL DEFAULT 0,
    CHECK (sentinel IN (0, 1))
);

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);
