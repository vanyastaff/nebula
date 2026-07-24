-- Migration 0040: add a material-authority epoch and structural refresh-retry gate.
--
-- The gate is deliberately separate from user metadata and refresh-claim TTL.
-- SQLite cannot add the cross-column CHECK in place, so rebuild the canonical
-- credential relation. SQLx owns the transaction.

CREATE TABLE credentials_0040 (
    id                            TEXT    NOT NULL PRIMARY KEY,
    name                          TEXT,
    owner_id                      TEXT    NOT NULL,
    credential_key                TEXT    NOT NULL,
    state_kind                    TEXT    NOT NULL,
    state_version                 INTEGER NOT NULL,
    data                          BLOB    NOT NULL,
    version                       INTEGER NOT NULL,
    material_epoch                INTEGER NOT NULL,
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL,
    expires_at                    INTEGER,
    reauth_required               INTEGER NOT NULL,
    metadata                      TEXT    NOT NULL,
    record_state                  TEXT    NOT NULL,
    tombstoned_at                 INTEGER,
    refresh_retry_mode            TEXT,
    refresh_retry_not_before      INTEGER,
    refresh_retry_phase           TEXT,
    refresh_retry_kind            TEXT,
    refresh_retry_diagnostic_code TEXT,

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
    CONSTRAINT credentials_material_epoch_range
        CHECK (
            typeof(material_epoch) = 'integer'
            AND material_epoch BETWEEN 1 AND 9223372036854775807
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
    CONSTRAINT credentials_refresh_retry_gate_shape
        CHECK (
            (
                refresh_retry_mode IS NULL
                AND refresh_retry_not_before IS NULL
                AND refresh_retry_phase IS NULL
                AND refresh_retry_kind IS NULL
                AND refresh_retry_diagnostic_code IS NULL
            )
            OR
            (
                record_state = 'live'
                AND refresh_retry_mode IS NOT NULL
                AND refresh_retry_phase IS NOT NULL
                AND refresh_retry_phase IN (
                    'before_dispatch',
                    'provider_confirmed_not_applied'
                )
                AND refresh_retry_kind IS NOT NULL
                AND refresh_retry_kind IN (
                    'transient_network',
                    'provider_unavailable',
                    'protocol_error'
                )
                AND (
                    refresh_retry_diagnostic_code IS NULL
                    OR (
                        typeof(refresh_retry_diagnostic_code) = 'text'
                        AND length(refresh_retry_diagnostic_code) BETWEEN 1 AND 64
                        AND refresh_retry_diagnostic_code
                            NOT GLOB '*[^A-Za-z0-9_.:-]*'
                    )
                )
                AND (
                    (
                        refresh_retry_mode = 'never'
                        AND refresh_retry_not_before IS NULL
                    )
                    OR
                    (
                        refresh_retry_mode = 'not_before'
                        AND refresh_retry_not_before IS NOT NULL
                        AND typeof(refresh_retry_not_before) = 'integer'
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
                AND refresh_retry_mode IS NULL
                AND refresh_retry_not_before IS NULL
                AND refresh_retry_phase IS NULL
                AND refresh_retry_kind IS NULL
                AND refresh_retry_diagnostic_code IS NULL
            )
        )
);

INSERT INTO credentials_0040 (
    id,
    name,
    owner_id,
    credential_key,
    state_kind,
    state_version,
    data,
    version,
    material_epoch,
    created_at,
    updated_at,
    expires_at,
    reauth_required,
    metadata,
    record_state,
    tombstoned_at,
    refresh_retry_mode,
    refresh_retry_not_before,
    refresh_retry_phase,
    refresh_retry_kind,
    refresh_retry_diagnostic_code
)
SELECT
    id,
    name,
    owner_id,
    credential_key,
    state_kind,
    state_version,
    data,
    version,
    1,
    created_at,
    updated_at,
    expires_at,
    reauth_required,
    metadata,
    record_state,
    tombstoned_at,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL
FROM credentials;

DROP TABLE credentials;

ALTER TABLE credentials_0040 RENAME TO credentials;

CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name);

CREATE INDEX idx_credentials_state_kind
    ON credentials(state_kind);

CREATE INDEX idx_credentials_expiring
    ON credentials(expires_at)
    WHERE expires_at IS NOT NULL;
