-- 0011: Executions
-- Layer: Execution
-- Spec: 16 (storage-schema), 08 (cancellation), 17 (multi-process)
--
-- Central execution entity: inbox + run + archive.
-- CAS via `version`; lease via `claimed_by`/`claimed_until`.

CREATE TABLE executions (
    id                     BYTEA PRIMARY KEY,        -- exe_ ULID
    workspace_id           BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    org_id                 BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workflow_version_id    BYTEA NOT NULL REFERENCES workflow_versions(id),
    status                 TEXT NOT NULL,             -- 'Pending'/'Queued'/'Running'/'Suspended'/'Succeeded'/'Failed'/'Cancelled'/'Cancelling'/'Orphaned'
    source                 JSONB NOT NULL,            -- ExecutionSource enum serialized
    input                  JSONB,                     -- trigger payload / manual input
    output                 JSONB,                     -- final workflow output
    vars                   JSONB,                     -- execution-wide $vars
    progress_summary       JSONB,                     -- {done: 5, running: 2, pending: 3}

    -- Timing
    created_at             TIMESTAMPTZ NOT NULL,
    scheduled_at           TIMESTAMPTZ,               -- for delayed starts
    started_at             TIMESTAMPTZ,
    finished_at            TIMESTAMPTZ,

    -- Claim / lease (multi-process coordination, spec 17)
    claimed_by             BYTEA,                     -- nbl_ instance_id of worker holding lease
    claimed_until          TIMESTAMPTZ,

    -- Cancel tracking (spec 08)
    cancel_requested_at    TIMESTAMPTZ,
    cancel_requested_by    BYTEA,
    cancel_reason          TEXT,
    escalated              BOOLEAN NOT NULL DEFAULT FALSE,

    -- Restart tracking
    restarted_from         BYTEA REFERENCES executions(id),

    -- Timeout
    execution_timeout_at   TIMESTAMPTZ,               -- computed: created_at + workflow timeout

    -- CAS
    version                BIGINT NOT NULL DEFAULT 0
);

-- Claim unclaimed pending/queued work
CREATE INDEX idx_executions_pending_claim
    ON executions (created_at)
    WHERE status IN ('Pending', 'Queued') AND claimed_until IS NULL;

-- Detect stale leases for takeover
CREATE INDEX idx_executions_stale_lease
    ON executions (claimed_until)
    WHERE status = 'Running' AND claimed_until IS NOT NULL;

-- Workspace listing (recent first)
CREATE INDEX idx_executions_workspace_list
    ON executions (workspace_id, created_at DESC);

-- FK lookup for workflow version
CREATE INDEX idx_executions_by_version
    ON executions (workflow_version_id);

-- Timeout scanner
CREATE INDEX idx_executions_timeout_check
    ON executions (execution_timeout_at)
    WHERE status = 'Running' AND execution_timeout_at IS NOT NULL;

-- Restart chain navigation
CREATE INDEX idx_executions_restart_chain
    ON executions (restarted_from)
    WHERE restarted_from IS NOT NULL;
