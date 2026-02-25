-- 014: Execution Lifecycle (idempotency + approvals)

-- ============================================================
-- IDEMPOTENCY KEYS
-- ============================================================

CREATE TABLE idempotency_keys (
    key             VARCHAR(512) PRIMARY KEY,
    execution_id    UUID REFERENCES executions(id) ON DELETE CASCADE,
    result          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_idem_keys_expires ON idempotency_keys(expires_at);

-- ============================================================
-- EXECUTION APPROVALS (human-in-the-loop, waiting state)
-- ============================================================

CREATE TYPE approval_status AS ENUM ('pending', 'approved', 'rejected', 'expired');

CREATE TABLE execution_approvals (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    execution_id    UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_run_id     UUID REFERENCES execution_node_runs(id) ON DELETE CASCADE,
    assignee_id     UUID REFERENCES users(id) ON DELETE SET NULL,
    status          approval_status NOT NULL DEFAULT 'pending',
    message         TEXT,
    response_note   TEXT,
    responded_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    responded_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_approvals_execution ON execution_approvals(execution_id);
CREATE INDEX idx_approvals_assignee ON execution_approvals(assignee_id) WHERE status = 'pending';
