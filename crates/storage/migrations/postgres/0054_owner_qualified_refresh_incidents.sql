-- Migration 0054: bind refresh claims and sentinel incidents to the credential
-- aggregate's canonical owner partition.
--
-- This is an aggregate-transforming migration.  A missing credential row makes
-- the NOT NULL step fail closed instead of inventing authority for an orphaned
-- claim or incident.  SQLx owns the transaction.

LOCK TABLE credentials, credential_refresh_claims, credential_sentinel_events
    IN ACCESS EXCLUSIVE MODE;

ALTER TABLE credential_refresh_claims
    ADD COLUMN owner_id TEXT;

UPDATE credential_refresh_claims AS claim
SET owner_id = credential.owner_id
FROM credentials AS credential
WHERE credential.id = claim.credential_id;

ALTER TABLE credential_refresh_claims
    ALTER COLUMN owner_id SET NOT NULL,
    DROP CONSTRAINT credential_refresh_claims_pkey,
    ADD CONSTRAINT credential_refresh_claims_pkey
        PRIMARY KEY (owner_id, credential_id);

ALTER TABLE credential_sentinel_events
    ADD COLUMN owner_id TEXT;

UPDATE credential_sentinel_events AS event
SET owner_id = credential.owner_id
FROM credentials AS credential
WHERE credential.id = event.credential_id;

ALTER TABLE credential_sentinel_events
    ALTER COLUMN owner_id SET NOT NULL;

DROP INDEX idx_sentinel_events_cred_time;

CREATE INDEX idx_sentinel_events_owner_cred_time
    ON credential_sentinel_events(owner_id, credential_id, detected_at);
