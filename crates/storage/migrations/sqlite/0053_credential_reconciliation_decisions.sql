-- Operator reconciliation of a poisoned refresh claim.
--
-- A sentinel event is created by the reclaim sweep and, until this migration,
-- had no record of *how* the ambiguous provider outcome was resolved. These
-- columns are the resolution record: they are written by
-- `RefreshClaimAdjudicator::adjudicate`, which keys on the poisoned claim row
-- and creates the incident here when the sweep has not run yet.
--
-- `adjudicated_at IS NULL` means "no provider outcome is known for this
-- incident", which is the state that retains fail-closed poison. Resolving an
-- incident never changes the count of incidents in the sentinel window: the
-- threshold is resolution-blind by design.
--
-- Plain nullable adds, exactly as 0039 extended this same table. Every column
-- is NULL for incidents recorded before this migration, and those stay
-- unresolved. The closed decision spelling and the digest width are not
-- repeated here as CHECKs: `RefreshOutcomeDecision` and `[u8; 32]` own those
-- invariants, and only the adapter writes these columns.

ALTER TABLE credential_sentinel_events
    ADD COLUMN adjudicated_at INTEGER;

ALTER TABLE credential_sentinel_events
    ADD COLUMN adjudication_decision TEXT;

ALTER TABLE credential_sentinel_events
    ADD COLUMN adjudication_evidence TEXT;

ALTER TABLE credential_sentinel_events
    ADD COLUMN adjudication_evidence_digest BLOB;
