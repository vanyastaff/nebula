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
    workspace_id TEXT NOT NULL CHECK (length(workspace_id) > 0),
    org_id TEXT NOT NULL CHECK (length(org_id) > 0),
    execution_id TEXT NOT NULL CHECK (length(execution_id) > 0),
    node_key TEXT NOT NULL CHECK (length(node_key) > 0),
    action_key TEXT NOT NULL CHECK (length(action_key) > 0),
    action_version TEXT NOT NULL CHECK (length(action_version) > 0),

    -- The next iteration to run; the stateful runtime caps a loop at 10000.
    iteration INTEGER NOT NULL
        CHECK (typeof(iteration) = 'integer' AND iteration BETWEEN 1 AND 10000),
    -- Canonical JSON bytes of the action's state, at most 1 MiB.
    state BLOB NOT NULL
        CHECK (typeof(state) = 'blob' AND length(state) <= 1048576),
    -- SHA-256 of `state`: the identity of an exact recommit.
    state_digest BLOB NOT NULL
        CHECK (typeof(state_digest) = 'blob' AND length(state_digest) = 32),
    resume_delay_ms INTEGER
        CHECK (resume_delay_ms IS NULL OR (typeof(resume_delay_ms) = 'integer' AND resume_delay_ms >= 0)),
    -- Distinct iterated ledger positions below `iteration` when written.
    attested_positions INTEGER NOT NULL
        CHECK (typeof(attested_positions) = 'integer' AND attested_positions >= 0),
    attempt_generation INTEGER NOT NULL
        CHECK (typeof(attempt_generation) = 'integer' AND attempt_generation >= 0),
    fencing_generation INTEGER NOT NULL
        CHECK (typeof(fencing_generation) = 'integer' AND fencing_generation >= 0),
    written_at_ms INTEGER NOT NULL
        CHECK (typeof(written_at_ms) = 'integer'),

    PRIMARY KEY (workspace_id, org_id, execution_id, node_key, action_key, action_version),
    FOREIGN KEY (execution_id, workspace_id, org_id)
        REFERENCES port_executions (id, workspace_id, org_id) ON DELETE CASCADE
);
