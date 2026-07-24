-- Migration 0039: make credential ownership, terminal state, and refresh
-- incident identity structural.
--
-- Readiness admission validates the canonical SQLx ledger and every legacy
-- row before SQLx starts this migration. The explicit table lock protects the
-- rewrite from concurrent legacy writers after the quiescence boundary.
-- SQLx owns the transaction; do not add transaction control statements here.
-- Historical sentinel events remain nullable because no trustworthy claim UUID
-- can be reconstructed; all post-0039 accounting writes the incident key.

LOCK TABLE credentials IN ACCESS EXCLUSIVE MODE;

ALTER TABLE credential_sentinel_events
    ADD COLUMN claim_id UUID;

CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id
    ON credential_sentinel_events(claim_id)
    WHERE claim_id IS NOT NULL;

ALTER TABLE credentials
    ADD COLUMN record_state TEXT,
    ADD COLUMN tombstoned_at TIMESTAMPTZ;

UPDATE credentials
SET
    name = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN NULL
        WHEN name IS NOT NULL THEN name
        ELSE metadata::jsonb #>> '{display,display_name}'
    END,
    data = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN ''::bytea
        ELSE data
    END,
    expires_at = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN NULL
        ELSE expires_at
    END,
    reauth_required = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN FALSE
        ELSE reauth_required
    END,
    metadata = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN '{}'
        ELSE metadata
    END,
    record_state = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN 'tombstoned'
        ELSE 'live'
    END,
    tombstoned_at = CASE
        WHEN metadata::jsonb ? 'revoked_at' THEN updated_at
        ELSE NULL
    END;

ALTER TABLE credentials
    ALTER COLUMN owner_id SET NOT NULL,
    ALTER COLUMN record_state SET NOT NULL,
    ADD CONSTRAINT credentials_state_version_range
        CHECK (state_version BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT credentials_version_range
        CHECK (version BETWEEN 1 AND 9223372036854775807),
    ADD CONSTRAINT credentials_metadata_object
        CHECK (metadata IS JSON OBJECT WITH UNIQUE KEYS),
    ADD CONSTRAINT credentials_live_name_projection
        CHECK (
            record_state = 'tombstoned'
            OR (
                record_state = 'live'
                AND (
                    (
                        name IS NULL
                        AND (
                            metadata::jsonb #> '{display,display_name}' IS NULL
                            OR jsonb_typeof(metadata::jsonb #> '{display,display_name}') = 'null'
                        )
                    )
                    OR (
                        name IS NOT NULL
                        AND jsonb_typeof(metadata::jsonb #> '{display,display_name}') = 'string'
                        AND name = metadata::jsonb #>> '{display,display_name}'
                    )
                )
            )
        ),
    ADD CONSTRAINT credentials_record_shape
        CHECK (
            (
                record_state = 'live'
                AND tombstoned_at IS NULL
                AND version <= 9223372036854775806
            )
            OR
            (
                record_state = 'tombstoned'
                AND tombstoned_at IS NOT NULL
                AND octet_length(data) = 0
                AND name IS NULL
                AND expires_at IS NULL
                AND reauth_required = FALSE
                AND metadata = '{}'
            )
        );

DROP INDEX idx_credentials_owner_name;

CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name);
