-- Aggregate-neutral additive ownership markers. No historical lease/queue backfill.
-- Recovery guarantees apply only to acceptors writing this marker atomically.
CREATE TABLE port_execution_turn_acceptances (
    execution_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    last_accepted_fencing_generation INTEGER NOT NULL
        CHECK (typeof(last_accepted_fencing_generation) = 'integer' AND last_accepted_fencing_generation > 0),
    source_kind TEXT NOT NULL CHECK (
        source_kind IN ('Job', 'ControlStart', 'ControlResume', 'ControlRestart')
    ),
    source_queue_id BLOB NOT NULL CHECK (typeof(source_queue_id) = 'blob' AND length(source_queue_id) = 16),
    FOREIGN KEY (execution_id, workspace_id, org_id)
        REFERENCES port_executions (id, workspace_id, org_id) ON DELETE CASCADE
);
CREATE INDEX idx_execution_refs_recovery_cursor
    ON port_execution_revision_refs (worker_flavor_id, execution_id)
    WHERE reference_state = 'live';
