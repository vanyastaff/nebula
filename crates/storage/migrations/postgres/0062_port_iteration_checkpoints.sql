-- Durable, fenced iteration checkpoints of journaled stateful actions.
--
-- One row per (tenant, execution, node, action key, action version): the next
-- iteration a stateful action's loop runs and the state it runs it with,
-- written only under the execution's live lease after that iteration's
-- effect barrier passed. A redeployed action version reads no row (it starts
-- at iteration 0); the row goes with its execution (ON DELETE CASCADE).
--
-- Aggregate-neutral: an empty relation. Nothing is inspected, inferred or
-- backfilled; a missing row means "replay from iteration 0", the behaviour
-- every stateful node had before this table existed.

CREATE TABLE port_iteration_checkpoints (
    -- Tenant scope is constrained only as `port_executions` constrains it
    -- (the FK below): any execution admitted there can be checkpointed.
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    execution_id TEXT NOT NULL CHECK (length(execution_id) > 0),
    node_key TEXT NOT NULL CHECK (length(node_key) > 0),
    action_key TEXT NOT NULL CHECK (length(action_key) > 0),
    -- The version text has no length bound (an admitted build suffix may be
    -- long), so it is stored but never indexed; the key carries its SHA-256.
    action_version TEXT NOT NULL CHECK (length(action_version) > 0),
    action_version_digest BYTEA NOT NULL CHECK (octet_length(action_version_digest) = 32),

    -- The next iteration to run; the stateful runtime caps a loop at 10000.
    iteration INTEGER NOT NULL CHECK (iteration BETWEEN 1 AND 10000),
    -- Canonical JSON bytes of the action's state, at most 1 MiB.
    state BYTEA NOT NULL CHECK (octet_length(state) <= 1048576),
    -- SHA-256 of `state`: the identity of an exact recommit.
    state_digest BYTEA NOT NULL CHECK (octet_length(state_digest) = 32),
    resume_delay_ms BIGINT CHECK (resume_delay_ms IS NULL OR resume_delay_ms >= 0),
    -- Distinct iterated ledger positions below `iteration` when written.
    attested_positions INTEGER NOT NULL CHECK (attested_positions >= 0),
    attempt_generation BIGINT NOT NULL CHECK (attempt_generation >= 0),
    fencing_generation BIGINT NOT NULL CHECK (fencing_generation >= 0),
    written_at_ms BIGINT NOT NULL,

    PRIMARY KEY (workspace_id, org_id, execution_id, node_key, action_key, action_version_digest),
    FOREIGN KEY (execution_id, workspace_id, org_id)
        REFERENCES port_executions (id, workspace_id, org_id) ON DELETE CASCADE
);
