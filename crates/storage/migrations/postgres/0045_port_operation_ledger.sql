-- Durable operation ledger for the remote-effect protocol.
--
-- One row per intended remote-effect occurrence. The row exists *before* the
-- provider is invoked, so a worker that dies mid-call leaves behind a record
-- saying an effect may have crossed the boundary — which is the whole point.
--
-- A slot is addressed two ways on purpose: by its storage-minted identity, and
-- by the natural key a caller can reconstruct without having seen the slot
-- before. The natural key is what lets a restarted worker find the operation it
-- already prepared; the minted identity is what a caller carries afterwards.

CREATE TABLE port_operation_ledger (
    slot_id BYTEA PRIMARY KEY
        CHECK (octet_length(slot_id) = 16),

    workspace_id TEXT NOT NULL CHECK (length(workspace_id) > 0),
    org_id TEXT NOT NULL CHECK (length(org_id) > 0),
    execution_id TEXT NOT NULL CHECK (length(execution_id) > 0),
    node_key TEXT NOT NULL CHECK (length(node_key) > 0),
    -- Distinguishes two intended occurrences from one node. Identical payloads
    -- are still two effects when the occurrence differs.
    occurrence TEXT NOT NULL CHECK (length(occurrence) > 0),

    attempt_generation BIGINT NOT NULL CHECK (attempt_generation >= 0),

    -- Canonicalization version travels with the digest: digests produced under
    -- different rules are not comparable, so a version change must read as a
    -- mismatch rather than risk reusing an operation identity for a different
    -- request.
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version >= 0),
    fingerprint BYTEA NOT NULL CHECK (octet_length(fingerprint) = 32),

    -- The guarantee recorded at prepare time, not the one the destination
    -- happens to offer at recovery time.
    destination TEXT NOT NULL
        CHECK (destination IN ('stable_key', 'reconcilable', 'opaque')),

    operation_id BYTEA NOT NULL UNIQUE
        CHECK (octet_length(operation_id) = 16),

    state TEXT NOT NULL
        CHECK (state IN ('prepared', 'succeeded', 'failed', 'outcome_unknown')),

    prepared_at_ms BIGINT NOT NULL,
    outcome_at_ms BIGINT,

    -- Operator-supplied, secret-free note recording why an ambiguous outcome
    -- became known. Present only for an adjudicated row.
    adjudication_evidence TEXT
        CHECK (adjudication_evidence IS NULL OR length(adjudication_evidence) > 0),
    adjudicated_at_ms BIGINT,

    -- A prepared row has no outcome timestamp; a resolved one always does.
    -- Without this a row could claim an outcome nothing ever recorded.
    CONSTRAINT port_operation_ledger_outcome_shape
        CHECK (
            (state = 'prepared' AND outcome_at_ms IS NULL)
            OR (state <> 'prepared' AND outcome_at_ms IS NOT NULL)
        ),

    -- Adjudication evidence and its timestamp travel together, and only on a
    -- determined outcome: adjudication resolves uncertainty, so it can never
    -- leave the row still unknown.
    CONSTRAINT port_operation_ledger_adjudication_shape
        CHECK (
            (adjudication_evidence IS NULL AND adjudicated_at_ms IS NULL)
            OR (
                adjudication_evidence IS NOT NULL
                AND adjudicated_at_ms IS NOT NULL
                AND state IN ('succeeded', 'failed')
            )
        ),

    -- The natural key a caller can rebuild without having seen the slot.
    -- Scope columns are inside it, so one tenant can neither collide with nor
    -- probe another's slots.
    CONSTRAINT port_operation_ledger_slot_identity
        UNIQUE (workspace_id, org_id, execution_id, node_key, occurrence)
);

-- Tenant-scoped lookup by the natural key drives every prepare.
CREATE INDEX idx_port_operation_ledger_execution
    ON port_operation_ledger (workspace_id, org_id, execution_id);

-- Operators sweep unresolved ambiguity; this is the query that finds it.
CREATE INDEX idx_port_operation_ledger_unresolved
    ON port_operation_ledger (workspace_id, org_id, prepared_at_ms)
    WHERE state IN ('prepared', 'outcome_unknown');
