-- ============================================================
-- 011: Workflow Sharing (cross-organization)
-- ============================================================

CREATE TABLE workflow_shares (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    shared_with     UUID REFERENCES organizations(id) ON DELETE CASCADE,   -- NULL = public
    permission      VARCHAR(32) NOT NULL DEFAULT 'read',   -- read | clone | execute
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (workflow_id, shared_with)
);

CREATE INDEX idx_shares_workflow ON workflow_shares(workflow_id);
