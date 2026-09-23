-- Migration 0054: bind refresh claims and sentinel incidents to the credential
-- aggregate's canonical owner partition.
--
-- SQLite cannot strengthen the existing relations in place, so both are
-- rebuilt.  The scalar owner lookup produces NULL for an orphan and the target
-- NOT NULL constraint aborts the whole SQLx transaction rather than inventing
-- tenant authority.

CREATE TABLE credential_refresh_claims_0054 (
    owner_id         TEXT    NOT NULL,
    credential_id     TEXT    NOT NULL,
    claim_id          TEXT    NOT NULL,
    generation        INTEGER NOT NULL,
    holder_replica_id TEXT    NOT NULL,
    acquired_at       INTEGER NOT NULL,
    expires_at        INTEGER NOT NULL,
    sentinel          INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_id, credential_id),
    CHECK (sentinel IN (0, 1))
);

INSERT INTO credential_refresh_claims_0054 (
    owner_id, credential_id, claim_id, generation, holder_replica_id,
    acquired_at, expires_at, sentinel
)
SELECT
    (SELECT credential.owner_id FROM credentials AS credential
     WHERE credential.id = claim.credential_id),
    credential_id, claim_id, generation, holder_replica_id,
    acquired_at, expires_at, sentinel
FROM credential_refresh_claims AS claim;

DROP TABLE credential_refresh_claims;
ALTER TABLE credential_refresh_claims_0054 RENAME TO credential_refresh_claims;

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);

CREATE TABLE credential_sentinel_events_0054 (
    id                              INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id                        TEXT    NOT NULL,
    credential_id                   TEXT    NOT NULL,
    detected_at                     INTEGER NOT NULL,
    crashed_holder                  TEXT    NOT NULL,
    generation                      INTEGER NOT NULL,
    claim_id                        TEXT,
    adjudicated_at                  INTEGER,
    adjudication_decision           TEXT,
    adjudication_evidence           TEXT,
    adjudication_evidence_digest    BLOB
);

INSERT INTO credential_sentinel_events_0054 (
    id, owner_id, credential_id, detected_at, crashed_holder, generation,
    claim_id, adjudicated_at, adjudication_decision, adjudication_evidence,
    adjudication_evidence_digest
)
SELECT
    id,
    (SELECT credential.owner_id FROM credentials AS credential
     WHERE credential.id = event.credential_id),
    credential_id, detected_at, crashed_holder, generation,
    claim_id, adjudicated_at, adjudication_decision, adjudication_evidence,
    adjudication_evidence_digest
FROM credential_sentinel_events AS event;

DROP TABLE credential_sentinel_events;
ALTER TABLE credential_sentinel_events_0054 RENAME TO credential_sentinel_events;

CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id
    ON credential_sentinel_events(claim_id)
    WHERE claim_id IS NOT NULL;

CREATE INDEX idx_sentinel_events_owner_cred_time
    ON credential_sentinel_events(owner_id, credential_id, detected_at);
