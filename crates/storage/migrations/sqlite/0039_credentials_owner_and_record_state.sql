-- Migration 0039: make credential ownership, terminal state, and refresh
-- incident identity structural.
--
-- Readiness admission runs before this migration and rejects rows that cannot
-- satisfy the final schema. SQLite cannot strengthen NOT NULL and CHECK
-- constraints in place, so use the canonical safe rebuild sequence:
-- CREATE -> INSERT SELECT -> DROP -> RENAME. SQLx owns the transaction.
-- Historical sentinel events remain nullable because no trustworthy claim UUID
-- can be reconstructed; all post-0039 accounting writes the incident key.

ALTER TABLE credential_sentinel_events
    ADD COLUMN claim_id TEXT;

CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id
    ON credential_sentinel_events(claim_id)
    WHERE claim_id IS NOT NULL;

CREATE TABLE credentials_0039 (
    id              TEXT    NOT NULL PRIMARY KEY,
    name            TEXT,
    owner_id        TEXT    NOT NULL,
    credential_key  TEXT    NOT NULL,
    state_kind      TEXT    NOT NULL,
    state_version   INTEGER NOT NULL,
    data            BLOB    NOT NULL,
    version         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    expires_at      INTEGER,
    reauth_required INTEGER NOT NULL,
    metadata        TEXT    NOT NULL,
    record_state    TEXT    NOT NULL,
    tombstoned_at   INTEGER,

    CONSTRAINT credentials_state_version_range
        CHECK (
            typeof(state_version) = 'integer'
            AND state_version BETWEEN 0 AND 4294967295
        ),
    CONSTRAINT credentials_version_range
        CHECK (
            typeof(version) = 'integer'
            AND version BETWEEN 1 AND 9223372036854775807
        ),
    CONSTRAINT credentials_reauth_boolean
        CHECK (
            typeof(reauth_required) = 'integer'
            AND reauth_required IN (0, 1)
        ),
    CONSTRAINT credentials_data_blob
        CHECK (typeof(data) = 'blob'),
    CONSTRAINT credentials_metadata_object
        CHECK (
            typeof(metadata) = 'text'
            AND json_valid(metadata)
            AND json_type(metadata) = 'object'
        ),
    CONSTRAINT credentials_live_name_projection
        CHECK (
            record_state = 'tombstoned'
            OR (
                record_state = 'live'
                AND (
                    (
                        name IS NULL
                        AND (
                            json_type(metadata, '$.display.display_name') IS NULL
                            OR json_type(metadata, '$.display.display_name') = 'null'
                        )
                    )
                    OR (
                        name IS NOT NULL
                        AND json_type(metadata, '$.display.display_name') = 'text'
                        AND name = json_extract(metadata, '$.display.display_name')
                    )
                )
            )
        ),
    CONSTRAINT credentials_record_shape
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
                AND length(data) = 0
                AND name IS NULL
                AND expires_at IS NULL
                AND reauth_required = 0
                AND metadata = '{}'
            )
        )
);

INSERT INTO credentials_0039 (
    id,
    name,
    owner_id,
    credential_key,
    state_kind,
    state_version,
    data,
    version,
    created_at,
    updated_at,
    expires_at,
    reauth_required,
    metadata,
    record_state,
    tombstoned_at
)
SELECT
    id,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN NULL
        WHEN name IS NOT NULL THEN name
        WHEN json_type(metadata, '$.display.display_name') = 'text'
            THEN json_extract(metadata, '$.display.display_name')
        ELSE NULL
    END,
    owner_id,
    credential_key,
    state_kind,
    state_version,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN zeroblob(0)
        ELSE data
    END,
    version,
    created_at,
    updated_at,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN NULL
        ELSE expires_at
    END,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN 0
        ELSE reauth_required
    END,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN '{}'
        ELSE metadata
    END,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN 'tombstoned'
        ELSE 'live'
    END,
    CASE
        WHEN json_type(metadata, '$.revoked_at') IS NOT NULL THEN updated_at
        ELSE NULL
    END
FROM credentials;

DROP TABLE credentials;

ALTER TABLE credentials_0039 RENAME TO credentials;

CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name);

CREATE INDEX idx_credentials_state_kind
    ON credentials(state_kind);

CREATE INDEX idx_credentials_expiring
    ON credentials(expires_at)
    WHERE expires_at IS NOT NULL;
