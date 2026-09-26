-- Migration 0061: add the credential admission epoch (the use revision).
--
-- The epoch advances in the same transaction as every write that closes
-- credential use: an advancing material replacement, a reauthentication
-- change, a won revoke claim, a provider-egress sentinel, and threshold
-- escalation. A consumer that bound the credential at one epoch must not keep
-- using it at another.
--
-- History is not guessed: every existing row starts at 1, so a binding made
-- before this cutover never matches a post-cutover observation. The backfill
-- default is dropped in the same migration, so an old writer's insert fails
-- instead of creating a row at a guessed epoch; its updates would not advance
-- the epoch, so old credential writers must be stopped before this applies.

ALTER TABLE credentials
    ADD COLUMN admission_epoch BIGINT NOT NULL DEFAULT 1;

ALTER TABLE credentials
    ALTER COLUMN admission_epoch DROP DEFAULT,
    ADD CONSTRAINT credentials_admission_epoch_range
        CHECK (admission_epoch BETWEEN 1 AND 9223372036854775807);
