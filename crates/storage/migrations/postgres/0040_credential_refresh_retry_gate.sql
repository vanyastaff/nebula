-- Migration 0040: add a material-authority epoch and structural refresh-retry gate.
--
-- The gate is deliberately separate from user metadata and refresh-claim TTL.
-- SQLx owns the transaction. Runtime gate deadlines use clock_timestamp(),
-- never transaction-start CURRENT_TIMESTAMP, after row-lock waits.

ALTER TABLE credentials
    ADD COLUMN material_epoch BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN refresh_retry_mode TEXT,
    ADD COLUMN refresh_retry_not_before TIMESTAMPTZ,
    ADD COLUMN refresh_retry_phase TEXT,
    ADD COLUMN refresh_retry_kind TEXT,
    ADD COLUMN refresh_retry_diagnostic_code TEXT,
    ADD CONSTRAINT credentials_refresh_retry_gate_shape
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
                    OR (refresh_retry_diagnostic_code COLLATE "C")
                        ~ '^[A-Za-z0-9_.:-]{1,64}$'
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
                    )
                )
            )
        );

ALTER TABLE credentials
    ALTER COLUMN material_epoch DROP DEFAULT,
    ADD CONSTRAINT credentials_material_epoch_range
        CHECK (material_epoch BETWEEN 1 AND 9223372036854775807);

ALTER TABLE credentials
    DROP CONSTRAINT credentials_record_shape,
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
                AND refresh_retry_mode IS NULL
                AND refresh_retry_not_before IS NULL
                AND refresh_retry_phase IS NULL
                AND refresh_retry_kind IS NULL
                AND refresh_retry_diagnostic_code IS NULL
            )
        );
