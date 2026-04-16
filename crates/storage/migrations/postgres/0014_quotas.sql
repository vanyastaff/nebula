-- 0014: Quotas and rate limits
-- Layer: Quotas
-- Spec: 16 (storage-schema), 10 (timeouts-quotas)

-- ── Org quota limits (per plan tier) ───────────────────────

CREATE TABLE org_quotas (
    org_id                          BYTEA PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    plan                            TEXT NOT NULL,
    concurrent_executions_limit     INT NOT NULL DEFAULT 50,
    executions_per_month_limit      BIGINT,
    active_workflows_limit          INT,
    total_workflows_limit           INT,
    workspaces_limit                INT,
    org_members_limit               INT,
    service_accounts_limit          INT,
    storage_bytes_limit             BIGINT,
    updated_at                      TIMESTAMPTZ NOT NULL
);

-- ── Org quota usage (atomic CAS counters) ──────────────────

CREATE TABLE org_quota_usage (
    org_id                          BYTEA PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    concurrent_executions           INT NOT NULL DEFAULT 0,
    active_workflows                INT NOT NULL DEFAULT 0,
    total_workflows                 INT NOT NULL DEFAULT 0,
    workspaces                      INT NOT NULL DEFAULT 0,
    org_members                     INT NOT NULL DEFAULT 0,
    service_accounts                INT NOT NULL DEFAULT 0,
    storage_bytes                   BIGINT NOT NULL DEFAULT 0,
    executions_this_month           BIGINT NOT NULL DEFAULT 0,
    month_reset_at                  TIMESTAMPTZ NOT NULL,
    updated_at                      TIMESTAMPTZ NOT NULL
);

-- ── Workspace quota usage ──────────────────────────────────

CREATE TABLE workspace_quota_usage (
    workspace_id                    BYTEA PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    concurrent_executions           INT NOT NULL DEFAULT 0,
    active_workflows                INT NOT NULL DEFAULT 0,
    updated_at                      TIMESTAMPTZ NOT NULL
);

-- ── Fair scheduling state (spec 17) ────────────────────────

CREATE TABLE workspace_dispatch_state (
    workspace_id        BYTEA PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    last_dispatched_at  TIMESTAMPTZ NOT NULL
);
