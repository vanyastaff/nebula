-- Migration 0061: add the credential admission epoch (the use revision).
--
-- The epoch advances in the same transaction as every write that closes
-- credential use: an advancing material replacement, a reauthentication
-- change, a won revoke claim, a provider-egress sentinel, and threshold
-- escalation. A consumer that bound the credential at one epoch must not keep
-- using it at another.
--
-- History is not guessed: every existing row starts at 1, so a binding made
-- before this cutover never matches a post-cutover observation. SQLite cannot
-- drop a column default without rebuilding the relation, so the backfill
-- default remains; one SQLite process owns its database, and old credential
-- writers must be stopped before this applies. SQLite checks the named range
-- constraint against every existing row as the column is added.

ALTER TABLE credentials
    ADD COLUMN admission_epoch INTEGER NOT NULL DEFAULT 1
        CONSTRAINT credentials_admission_epoch_range
        CHECK (
            typeof(admission_epoch) = 'integer'
            AND admission_epoch BETWEEN 1 AND 9223372036854775807
        );
