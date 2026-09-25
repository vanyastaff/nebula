-- Migration 0057: make provider-operation claims and incidents typed.
--
-- Historical sentinel rows can represent refresh or revoke and therefore
-- cannot be truthfully inferred. They remain fail-closed as
-- `legacy_unclassified`. New rows have no SQL default: an old writer that
-- omits the operation kind fails instead of silently classifying a revoke as
-- refresh during a rolling upgrade.

CREATE TABLE credential_refresh_claims_0057 (
    owner_id                  TEXT    NOT NULL,
    credential_id             TEXT    NOT NULL,
    claim_id                  TEXT    NOT NULL,
    generation                INTEGER NOT NULL,
    holder_replica_id         TEXT    NOT NULL,
    acquired_at               INTEGER NOT NULL,
    expires_at                INTEGER NOT NULL,
    sentinel                  INTEGER NOT NULL,
    operation_kind            TEXT    NOT NULL,
    observed_material_epoch   INTEGER,
    PRIMARY KEY (owner_id, credential_id),
    CHECK (sentinel IN (0, 1)),
    CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    CHECK (
        (operation_kind = 'revoke' AND observed_material_epoch IS NOT NULL)
        OR (operation_kind IN ('refresh', 'legacy_unclassified')
            AND observed_material_epoch IS NULL)
    )
);

INSERT INTO credential_refresh_claims_0057 (
    owner_id, credential_id, claim_id, generation, holder_replica_id,
    acquired_at, expires_at, sentinel, operation_kind,
    observed_material_epoch
)
SELECT owner_id, credential_id, claim_id, generation, holder_replica_id,
       acquired_at, expires_at, sentinel, 'legacy_unclassified', NULL
FROM credential_refresh_claims;

DROP TABLE credential_refresh_claims;
ALTER TABLE credential_refresh_claims_0057 RENAME TO credential_refresh_claims;

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);

CREATE TABLE credential_sentinel_events_0057 (
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
    adjudication_evidence_digest    BLOB,
    operation_kind                  TEXT    NOT NULL,
    observed_material_epoch         INTEGER,
    CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    CHECK (
        (operation_kind = 'revoke' AND observed_material_epoch IS NOT NULL)
        OR (operation_kind IN ('refresh', 'legacy_unclassified')
            AND observed_material_epoch IS NULL)
    )
);

INSERT INTO credential_sentinel_events_0057 (
    id, owner_id, credential_id, detected_at, crashed_holder, generation,
    claim_id, adjudicated_at, adjudication_decision, adjudication_evidence,
    adjudication_evidence_digest, operation_kind, observed_material_epoch
)
SELECT id, owner_id, credential_id, detected_at, crashed_holder, generation,
       claim_id, adjudicated_at, adjudication_decision, adjudication_evidence,
       adjudication_evidence_digest, 'legacy_unclassified', NULL
FROM credential_sentinel_events;

DROP TABLE credential_sentinel_events;
ALTER TABLE credential_sentinel_events_0057 RENAME TO credential_sentinel_events;

CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id
    ON credential_sentinel_events(claim_id)
    WHERE claim_id IS NOT NULL;

CREATE INDEX idx_sentinel_events_owner_cred_time
    ON credential_sentinel_events(owner_id, credential_id, detected_at);
