-- Aggregate-neutral: empty backend-owned execution-control refusal receipts.
-- No historical outcome or source authority is inferred. The execution owner
-- writes each receipt and its journal row atomically under its aggregate lock.
-- The first snapshot for one source/generation/outcome is immutable; retries
-- acknowledge that receipt rather than claiming a new observed reason.
-- Source kind distinguishes queue claim generation from accepted-turn lease
-- generation; decision_key distinguishes each node/attempt admission refusal.
CREATE TABLE port_execution_control_observation_receipts (
    execution_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('control_queue', 'job_dispatch', 'control_accepted_turn', 'job_accepted_turn')),
    source_queue_id BYTEA NOT NULL CHECK (octet_length(source_queue_id) = 16),
    source_generation BIGINT NOT NULL CHECK (source_generation > 0),
    decision_key TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('fenced', 'flavor-mismatch', 'deferred', 'throttled')),
    expected_flavor_id BYTEA,
    actual_flavor_id BYTEA,
    CHECK ((outcome = 'flavor-mismatch' AND expected_flavor_id IS NOT NULL AND actual_flavor_id IS NOT NULL AND expected_flavor_id <> actual_flavor_id AND octet_length(expected_flavor_id) = 32 AND octet_length(actual_flavor_id) = 32)
        OR (outcome <> 'flavor-mismatch' AND expected_flavor_id IS NULL AND actual_flavor_id IS NULL)),
    PRIMARY KEY (execution_id, workspace_id, org_id, source_kind, source_queue_id, source_generation, decision_key, outcome),
    FOREIGN KEY (execution_id, workspace_id, org_id)
        REFERENCES port_executions (id, workspace_id, org_id) ON DELETE CASCADE
);
