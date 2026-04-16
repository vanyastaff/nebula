-- 0014: Quotas and rate limits
-- Layer: Quotas
-- Spec: 16 (storage-schema), 10 (timeouts-quotas)

CREATE TABLE org_quotas (
    org_id                          BLOB PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    plan                            TEXT NOT NULL,
    concurrent_executions_limit     INTEGER NOT NULL DEFAULT 50,
    executions_per_month_limit      INTEGER,
    active_workflows_limit          INTEGER,
    total_workflows_limit           INTEGER,
    workspaces_limit                INTEGER,
    org_members_limit               INTEGER,
    service_accounts_limit          INTEGER,
    storage_bytes_limit             INTEGER,
    updated_at                      TEXT NOT NULL
);

CREATE TABLE org_quota_usage (
    org_id                          BLOB PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    concurrent_executions           INTEGER NOT NULL DEFAULT 0,
    active_workflows                INTEGER NOT NULL DEFAULT 0,
    total_workflows                 INTEGER NOT NULL DEFAULT 0,
    workspaces                      INTEGER NOT NULL DEFAULT 0,
    org_members                     INTEGER NOT NULL DEFAULT 0,
    service_accounts                INTEGER NOT NULL DEFAULT 0,
    storage_bytes                   INTEGER NOT NULL DEFAULT 0,
    executions_this_month           INTEGER NOT NULL DEFAULT 0,
    month_reset_at                  TEXT NOT NULL,
    updated_at                      TEXT NOT NULL
);

CREATE TABLE workspace_quota_usage (
    workspace_id                    BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    concurrent_executions           INTEGER NOT NULL DEFAULT 0,
    active_workflows                INTEGER NOT NULL DEFAULT 0,
    updated_at                      TEXT NOT NULL
);

CREATE TABLE workspace_dispatch_state (
    workspace_id        BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    last_dispatched_at  TEXT NOT NULL
);
