-- Migration 0057: make provider-operation claims and incidents typed.
-- Historical rows are deliberately fail-closed because their operation kind
-- cannot be inferred. New columns have no default so mixed old-writer inserts
-- fail instead of silently misclassifying revokes as refreshes.

LOCK TABLE credential_refresh_claims, credential_sentinel_events
    IN ACCESS EXCLUSIVE MODE;

ALTER TABLE credential_refresh_claims
    ADD COLUMN operation_kind TEXT,
    ADD COLUMN observed_material_epoch BIGINT;

UPDATE credential_refresh_claims
SET operation_kind = 'legacy_unclassified';

ALTER TABLE credential_refresh_claims
    ALTER COLUMN operation_kind SET NOT NULL,
    ADD CONSTRAINT credential_refresh_claims_operation_kind_check
        CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    ADD CONSTRAINT credential_refresh_claims_operation_epoch_check
        CHECK (
            (operation_kind = 'revoke' AND observed_material_epoch IS NOT NULL)
            OR (operation_kind IN ('refresh', 'legacy_unclassified')
                AND observed_material_epoch IS NULL)
        );

ALTER TABLE credential_sentinel_events
    ADD COLUMN operation_kind TEXT,
    ADD COLUMN observed_material_epoch BIGINT;

UPDATE credential_sentinel_events
SET operation_kind = 'legacy_unclassified';

ALTER TABLE credential_sentinel_events
    ALTER COLUMN operation_kind SET NOT NULL,
    ADD CONSTRAINT credential_sentinel_events_operation_kind_check
        CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    ADD CONSTRAINT credential_sentinel_events_operation_epoch_check
        CHECK (
            (operation_kind = 'revoke' AND observed_material_epoch IS NOT NULL)
            OR (operation_kind IN ('refresh', 'legacy_unclassified')
                AND observed_material_epoch IS NULL)
        );
