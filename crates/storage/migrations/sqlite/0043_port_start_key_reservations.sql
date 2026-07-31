-- Durable reservation of an accepted start key.
--
-- Start acceptance previously created the execution row and enqueued the Start
-- command as two independent statements, with no record of the key the caller
-- sent. A retried request therefore minted a second execution id and created a
-- second execution, and a crash between the two writes left an execution no
-- consumer would ever drive.
--
-- The reservation is what makes a start key identify one accepted *command*:
-- it is inserted in the same transaction as the execution row and the Start
-- control row, so all three commit together or none of them do.
--
-- The primary key is scope-qualified, so two tenants never contend for a key
-- and neither can probe the other's reservations.
--
-- `fingerprint_version` travels with the digest because comparing digests
-- produced under different canonicalization rules is meaningless: a change to
-- what "the same request" means must read as a mismatch, not as a match
-- against bytes that happen to collide.

CREATE TABLE port_start_key_reservations (
    workspace_id        TEXT NOT NULL,
    org_id              TEXT NOT NULL,
    start_key           TEXT NOT NULL,
    fingerprint_version INTEGER NOT NULL
        CHECK (fingerprint_version >= 0),
    fingerprint         BLOB NOT NULL
        CHECK (length(fingerprint) = 32),
    execution_id        TEXT NOT NULL,
    created_at_ms       INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, org_id, start_key)
);

-- Retention sweeps scan by age across tenants; the primary key is
-- scope-leading and cannot serve that.
CREATE INDEX idx_port_start_key_reservations_age
    ON port_start_key_reservations (created_at_ms);
