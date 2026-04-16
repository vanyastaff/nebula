# Spec 16 — Storage schema (foundation)

> **Status:** draft
> **Canon target:** §11.5 (matrix update)
> **Depends on:** 02 (tenancy), 03 (auth), 04 (RBAC), 06 (IDs), 07 (slugs), 13 (workflow versions)
> **Depended on by:** all runtime specs

## Problem

This is the foundation. Every other spec interacts with this one. Getting schema wrong means migrations, constraint retrofits, performance problems. This spec is the authoritative reference for **every table, every index, every constraint** in Nebula v1.

Two backends supported:
- **SQLite** — self-host single-process, `BLOB` for 16-byte IDs
- **Postgres** — cloud multi-process, `BYTEA` or `UUID` for 16-byte IDs

Schema must be compatible between both (semi-automatic translation or SQL file per backend).

## Decision

**Full SQL schema listed here, consolidated from all other specs.** Organized by layer: identity → tenancy → workflows → execution → triggers → credentials → quotas → audit. Each table has purpose, columns, indexes, FKs, retention policy.

## Tables by layer

### Layer 1 — Identity

#### `users`

Purpose: registered humans in the system.

```sql
CREATE TABLE users (
    id                 BYTEA PRIMARY KEY,            -- user_ ULID
    email              TEXT NOT NULL,                -- lowercased, unique
    email_verified_at  TIMESTAMPTZ,
    display_name       TEXT NOT NULL,
    avatar_url         TEXT,
    password_hash      TEXT,                         -- argon2id encoded, NULL for OAuth-only
    created_at         TIMESTAMPTZ NOT NULL,
    last_login_at      TIMESTAMPTZ,
    locked_until       TIMESTAMPTZ,
    failed_login_count INT NOT NULL DEFAULT 0,
    mfa_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    mfa_secret         BYTEA,                         -- encrypted with master key
    version            BIGINT NOT NULL DEFAULT 0,    -- CAS
    deleted_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_users_email_active
    ON users (LOWER(email))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_users_locked
    ON users (locked_until)
    WHERE locked_until IS NOT NULL;
```

**Retention:** soft delete immediately, hard delete after 30 days via background job.

#### `oauth_links`

Purpose: link external OAuth accounts (Google, GitHub) to `users` row.

```sql
CREATE TABLE oauth_links (
    user_id            BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider           TEXT NOT NULL,        -- 'google' / 'github' / 'microsoft'
    provider_user_id   TEXT NOT NULL,
    provider_email     TEXT,
    linked_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (provider, provider_user_id)
);

CREATE INDEX idx_oauth_links_user ON oauth_links (user_id);
```

#### `sessions`

Purpose: active login sessions (browser cookies).

```sql
CREATE TABLE sessions (
    id               BYTEA PRIMARY KEY,       -- sess_ ULID
    user_id          BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at       TIMESTAMPTZ NOT NULL,
    last_active_at   TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    ip_address       INET,
    user_agent       TEXT,
    revoked_at       TIMESTAMPTZ
);

CREATE INDEX idx_sessions_user_active
    ON sessions (user_id)
    WHERE revoked_at IS NULL AND expires_at > NOW();

CREATE INDEX idx_sessions_cleanup
    ON sessions (expires_at)
    WHERE revoked_at IS NULL;
```

**Retention:** delete expired rows daily.

#### `personal_access_tokens`

Purpose: API tokens for CLI, CI, automation. Used by users and service accounts.

```sql
CREATE TABLE personal_access_tokens (
    id                BYTEA PRIMARY KEY,      -- pat_ ULID
    principal_kind    TEXT NOT NULL,          -- 'user' / 'service_account'
    principal_id      BYTEA NOT NULL,
    name              TEXT NOT NULL,
    prefix            TEXT NOT NULL,          -- first 12 chars of token for display
    hash              BYTEA NOT NULL,         -- sha256 of full token
    scopes            JSONB NOT NULL,         -- [] = full, or ['read', 'workflows', ...]
    created_at        TIMESTAMPTZ NOT NULL,
    last_used_at      TIMESTAMPTZ,
    expires_at        TIMESTAMPTZ,
    revoked_at        TIMESTAMPTZ
);

CREATE INDEX idx_pat_hash
    ON personal_access_tokens (hash)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_pat_principal
    ON personal_access_tokens (principal_kind, principal_id);
```

#### `verification_tokens`

Purpose: one-time tokens for email verification, password reset, invitations, MFA recovery.

```sql
CREATE TABLE verification_tokens (
    token_hash   BYTEA PRIMARY KEY,           -- sha256 of token value
    user_id      BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,               -- 'email_verification' / 'password_reset' / 'org_invite' / 'mfa_recovery'
    payload      JSONB,                        -- kind-specific data (invite details, etc.)
    created_at   TIMESTAMPTZ NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    consumed_at  TIMESTAMPTZ
);

CREATE INDEX idx_verification_user_kind
    ON verification_tokens (user_id, kind)
    WHERE consumed_at IS NULL;

CREATE INDEX idx_verification_cleanup
    ON verification_tokens (expires_at)
    WHERE consumed_at IS NULL;
```

**Retention:** delete consumed or expired rows daily.

### Layer 2 — Tenancy

#### `orgs`

Purpose: organizations — top-level tenant.

```sql
CREATE TABLE orgs (
    id             BYTEA PRIMARY KEY,          -- org_ ULID
    slug           TEXT NOT NULL,              -- globally unique, case-insensitive
    display_name   TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,             -- first user (not FK to preserve history)
    plan           TEXT NOT NULL,              -- 'self_host' / 'free' / 'team' / 'business' / 'enterprise'
    billing_email  TEXT,
    settings       JSONB NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_orgs_slug_active
    ON orgs (LOWER(slug))
    WHERE deleted_at IS NULL;
```

#### `workspaces`

```sql
CREATE TABLE workspaces (
    id             BYTEA PRIMARY KEY,          -- ws_ ULID
    org_id         BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    is_default     BOOLEAN NOT NULL DEFAULT FALSE,
    settings       JSONB NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_workspaces_org_slug
    ON workspaces (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;

-- Only one default workspace per org
CREATE UNIQUE INDEX idx_workspaces_org_default
    ON workspaces (org_id)
    WHERE is_default = TRUE AND deleted_at IS NULL;
```

#### `org_members` and `workspace_members`

```sql
CREATE TABLE org_members (
    org_id             BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,          -- 'user' / 'service_account'
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,          -- 'OrgOwner' / 'OrgAdmin' / 'OrgMember' / 'OrgBilling'
    invited_at         TIMESTAMPTZ NOT NULL,
    invited_by         BYTEA,
    accepted_at        TIMESTAMPTZ,
    PRIMARY KEY (org_id, principal_kind, principal_id)
);

CREATE INDEX idx_org_members_principal
    ON org_members (principal_kind, principal_id);

CREATE TABLE workspace_members (
    workspace_id       BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,          -- 'WorkspaceAdmin' / 'Editor' / 'Runner' / 'Viewer'
    added_at           TIMESTAMPTZ NOT NULL,
    added_by           BYTEA NOT NULL,
    PRIMARY KEY (workspace_id, principal_kind, principal_id)
);

CREATE INDEX idx_workspace_members_principal
    ON workspace_members (principal_kind, principal_id);
```

#### `service_accounts`

```sql
CREATE TABLE service_accounts (
    id             BYTEA PRIMARY KEY,          -- sa_ ULID
    org_id         BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    description    TEXT,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL REFERENCES users(id),
    disabled_at    TIMESTAMPTZ,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_sa_org_slug
    ON service_accounts (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;
```

### Layer 3 — Workflows

#### `workflows`

```sql
CREATE TABLE workflows (
    id                  BYTEA PRIMARY KEY,         -- wf_ ULID
    workspace_id        BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    description         TEXT,
    current_version_id  BYTEA NOT NULL,            -- FK added after workflow_versions
    state               TEXT NOT NULL,              -- 'Active' / 'Paused' / 'Archived'
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL,
    version             BIGINT NOT NULL DEFAULT 0,
    deleted_at          TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_workflows_workspace_slug
    ON workflows (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_workflows_state
    ON workflows (workspace_id, state)
    WHERE deleted_at IS NULL;
```

#### `workflow_versions`

```sql
CREATE TABLE workflow_versions (
    id                    BYTEA PRIMARY KEY,    -- wfv_ ULID
    workflow_id           BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    version_number        INT NOT NULL,
    definition            JSONB NOT NULL,
    schema_version        INT NOT NULL,
    state                 TEXT NOT NULL,        -- 'Draft' / 'Published' / 'Archived' / 'Deleted'
    created_at            TIMESTAMPTZ NOT NULL,
    created_by            BYTEA NOT NULL,
    description           TEXT,
    compiled_expressions  BYTEA,
    compiled_validation   BYTEA,
    pinned                BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (workflow_id, version_number)
);

CREATE UNIQUE INDEX idx_workflow_versions_published
    ON workflow_versions (workflow_id)
    WHERE state = 'Published';

CREATE INDEX idx_workflow_versions_by_workflow
    ON workflow_versions (workflow_id, version_number DESC);

-- Add FK from workflows after workflow_versions exists
ALTER TABLE workflows
    ADD CONSTRAINT fk_workflows_current_version
    FOREIGN KEY (current_version_id) REFERENCES workflow_versions(id);
```

### Layer 4 — Execution

#### `executions` — run entity (inbox + run + archive)

```sql
CREATE TABLE executions (
    id                     BYTEA PRIMARY KEY,      -- exec_ ULID
    workspace_id           BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    org_id                 BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workflow_version_id    BYTEA NOT NULL REFERENCES workflow_versions(id),
    status                 TEXT NOT NULL,          -- 'Pending' / 'Queued' / 'Running' / 'Suspended' / 'Succeeded' / 'Failed' / 'Cancelled' / 'Cancelling' / 'Orphaned'
    source                 JSONB NOT NULL,         -- ExecutionSource enum serialized
    input                  JSONB,                  -- trigger payload / manual input
    output                 JSONB,                  -- final workflow output
    vars                   JSONB,                  -- execution-wide $vars
    progress_summary       JSONB,                  -- {done: 5, running: 2, pending: 3}
    
    -- Timing
    created_at             TIMESTAMPTZ NOT NULL,
    scheduled_at           TIMESTAMPTZ,            -- for delayed starts
    started_at             TIMESTAMPTZ,
    finished_at            TIMESTAMPTZ,
    
    -- Claim / lease (multi-process coordination)
    claimed_by             BYTEA,                  -- node_id of worker holding lease
    claimed_until          TIMESTAMPTZ,
    
    -- Cancel tracking
    cancel_requested_at    TIMESTAMPTZ,
    cancel_requested_by    BYTEA,
    cancel_reason          TEXT,
    escalated              BOOLEAN NOT NULL DEFAULT FALSE,
    
    -- Restart tracking
    restarted_from         BYTEA REFERENCES executions(id),
    
    -- Timeout
    execution_timeout_at   TIMESTAMPTZ,            -- computed from created_at + workflow_version.execution_timeout
    
    -- CAS
    version                BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX idx_executions_pending_claim
    ON executions (created_at)
    WHERE status IN ('Pending', 'Queued') AND claimed_until IS NULL;

CREATE INDEX idx_executions_stale_lease
    ON executions (claimed_until)
    WHERE status = 'Running' AND claimed_until IS NOT NULL;

CREATE INDEX idx_executions_workspace_list
    ON executions (workspace_id, created_at DESC);

CREATE INDEX idx_executions_by_version
    ON executions (workflow_version_id);

CREATE INDEX idx_executions_timeout_check
    ON executions (execution_timeout_at)
    WHERE status = 'Running' AND execution_timeout_at IS NOT NULL;

CREATE INDEX idx_executions_restart_chain
    ON executions (restarted_from)
    WHERE restarted_from IS NOT NULL;
```

**Retention:** terminal executions kept per plan (default 90 days), then GC'd.

#### `execution_nodes` — per-attempt node details

```sql
CREATE TABLE execution_nodes (
    id                     BYTEA PRIMARY KEY,      -- node_ ULID
    execution_id           BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    logical_node_id        TEXT NOT NULL,           -- from workflow definition
    attempt                INT NOT NULL,            -- 1, 2, 3, ... per retry
    
    -- Status
    status                 TEXT NOT NULL,          -- 'Running' / 'Succeeded' / 'Failed' / 'Cancelled' / 'PendingRetry' / 'Suspended'
    started_at             TIMESTAMPTZ,
    finished_at            TIMESTAMPTZ,
    
    -- Input/output
    input                  JSONB,
    output                 JSONB,
    
    -- Error tracking
    error_kind             TEXT,                   -- 'Transient' / 'Permanent' / 'Cancelled' / 'Fatal' / 'Timeout'
    error_message          TEXT,
    error_retry_hint_ms    BIGINT,                 -- from TransientWithHint
    idempotency_key        TEXT NOT NULL,          -- {exec_id}:{logical_node_id}:{attempt}
    
    -- Retry tracking
    wake_at                TIMESTAMPTZ,            -- NULL unless PendingRetry or Suspended with Timer
    wake_signal_name       TEXT,                   -- NULL unless Suspended with Signal
    
    -- StatefulAction state (NULL for stateless)
    state                  JSONB,                  -- inline state ≤1 MB
    state_blob_ref         BYTEA,                  -- reference for larger state (v1.5)
    state_schema_hash      BYTEA,                  -- for schema migration detection
    iteration_count        INT NOT NULL DEFAULT 0,
    
    -- Cancel escalation
    escalated              BOOLEAN NOT NULL DEFAULT FALSE,
    
    -- CAS + lease ownership
    version                BIGINT NOT NULL DEFAULT 0,
    
    UNIQUE (execution_id, logical_node_id, attempt),
    UNIQUE (idempotency_key)
);

CREATE INDEX idx_execution_nodes_by_exec
    ON execution_nodes (execution_id, started_at);

CREATE INDEX idx_execution_nodes_pending_retry
    ON execution_nodes (wake_at)
    WHERE status = 'PendingRetry' AND wake_at IS NOT NULL;

CREATE INDEX idx_execution_nodes_suspended
    ON execution_nodes (wake_at, wake_signal_name)
    WHERE status = 'Suspended';
```

**Retention:** deleted together with parent execution via cascade.

#### `execution_journal` — append-only audit trail

```sql
CREATE TABLE execution_journal (
    id              BYTEA PRIMARY KEY,          -- ULID, monotonic for ordering
    execution_id    BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    sequence        BIGINT NOT NULL,             -- per-execution monotonic
    event_type      TEXT NOT NULL,               -- 'ExecutionStarted' / 'NodeStarted' / 'NodeFinished' / ...
    node_attempt_id BYTEA,                       -- NULL for execution-level events
    payload         JSONB NOT NULL,
    emitted_at      TIMESTAMPTZ NOT NULL,
    
    UNIQUE (execution_id, sequence)
);

CREATE INDEX idx_execution_journal_by_exec
    ON execution_journal (execution_id, sequence);
```

**Append-only**, no UPDATE or DELETE in runtime code. Retention via TRUNCATE of old executions.

#### `execution_control_queue` — cancel / run signals (outbox pattern)

```sql
CREATE TABLE execution_control_queue (
    id              BYTEA PRIMARY KEY,
    execution_id    BYTEA NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    command         TEXT NOT NULL,              -- 'Cancel' / 'Terminate' / 'Resume' / 'Restart'
    issued_by       BYTEA,                       -- user or service account
    issued_at       TIMESTAMPTZ NOT NULL,
    status          TEXT NOT NULL,               -- 'Pending' / 'Processing' / 'Completed' / 'Failed'
    processed_at    TIMESTAMPTZ,
    processed_by    BYTEA,                       -- node_id that processed
    error_message   TEXT
);

CREATE INDEX idx_execution_control_queue_pending
    ON execution_control_queue (execution_id, issued_at)
    WHERE status = 'Pending';
```

**Retention:** processed rows kept 7 days for audit, then GC'd.

### Layer 5 — Triggers

#### `triggers`

```sql
CREATE TABLE triggers (
    id             BYTEA PRIMARY KEY,          -- trig_ ULID
    workspace_id   BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    workflow_id    BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,              -- 'manual' / 'cron' / 'webhook' / 'event' / 'polling'
    config         JSONB NOT NULL,
    state          TEXT NOT NULL,              -- 'active' / 'paused' / 'archived'
    run_as         BYTEA,                       -- ServiceAccountId, NULL → workspace default
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_triggers_workspace_slug
    ON triggers (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_triggers_active
    ON triggers (workspace_id, state)
    WHERE state = 'active' AND deleted_at IS NULL;
```

#### `trigger_events` — inbox with dedup

```sql
CREATE TABLE trigger_events (
    id              BYTEA PRIMARY KEY,          -- evt_ ULID
    trigger_id      BYTEA NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    event_id        TEXT NOT NULL,              -- author-configured or fallback hash
    received_at     TIMESTAMPTZ NOT NULL,
    claim_state     TEXT NOT NULL,              -- 'pending' / 'claimed' / 'dispatched' / 'failed'
    claimed_by      BYTEA,                       -- dispatcher node_id
    claimed_at      TIMESTAMPTZ,
    payload         JSONB NOT NULL,
    execution_id    BYTEA,                       -- set after execution created
    metadata        JSONB,
    
    UNIQUE (trigger_id, event_id)               -- DEDUP ENFORCEMENT
);

CREATE INDEX idx_trigger_events_pending
    ON trigger_events (received_at)
    WHERE claim_state = 'pending';

CREATE INDEX idx_trigger_events_cleanup
    ON trigger_events (received_at)
    WHERE claim_state = 'dispatched';
```

**Retention:** dispatched rows kept 30 days for audit + dedup window, then GC'd.

#### `cron_fire_slots` — leaderless coordination

```sql
CREATE TABLE cron_fire_slots (
    trigger_id      BYTEA NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    scheduled_for   TIMESTAMPTZ NOT NULL,
    claimed_by      BYTEA NOT NULL,             -- dispatcher node_id
    claimed_at      TIMESTAMPTZ NOT NULL,
    execution_id    BYTEA,                       -- populated after execution created
    PRIMARY KEY (trigger_id, scheduled_for)
);

CREATE INDEX idx_cron_fire_slots_cleanup
    ON cron_fire_slots (claimed_at);
```

**Retention:** delete rows older than 7 days (keeps recent history for debugging).

#### `pending_signals`

```sql
CREATE TABLE pending_signals (
    id                BYTEA PRIMARY KEY,
    node_attempt_id   BYTEA NOT NULL REFERENCES execution_nodes(id) ON DELETE CASCADE,
    signal_name       TEXT NOT NULL,
    payload           JSONB,
    received_at       TIMESTAMPTZ NOT NULL,
    consumed_at       TIMESTAMPTZ
);

CREATE INDEX idx_pending_signals_unconsumed
    ON pending_signals (node_attempt_id, signal_name)
    WHERE consumed_at IS NULL;
```

### Layer 6 — Credentials and resources

#### `credentials`

```sql
CREATE TABLE credentials (
    id                  BYTEA PRIMARY KEY,      -- cred_ ULID
    org_id              BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id        BYTEA REFERENCES workspaces(id) ON DELETE CASCADE,  -- NULL for org-level
    slug                TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    kind                TEXT NOT NULL,          -- credential type (e.g., 'oauth2_google', 'api_key', 'basic_auth')
    scope               TEXT NOT NULL,          -- 'workspace' or 'org'
    encrypted_secret    BYTEA NOT NULL,         -- encrypted with org master key
    encryption_version  INT NOT NULL,            -- supports key rotation
    allowed_workspaces  BYTEA[],                -- for org-level: list of allowed ws_ids
    metadata            JSONB,                  -- non-secret data (client_id, scopes, etc.)
    created_at          TIMESTAMPTZ NOT NULL,
    created_by          BYTEA NOT NULL,
    last_rotated_at     TIMESTAMPTZ,
    last_used_at        TIMESTAMPTZ,
    version             BIGINT NOT NULL DEFAULT 0,
    deleted_at          TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_credentials_workspace_slug
    ON credentials (workspace_id, LOWER(slug))
    WHERE scope = 'workspace' AND deleted_at IS NULL;

CREATE UNIQUE INDEX idx_credentials_org_slug
    ON credentials (org_id, LOWER(slug))
    WHERE scope = 'org' AND deleted_at IS NULL;
```

**Important:** `encrypted_secret` stored as ciphertext. Decryption happens in `nebula-credential` only, using org-level master key (self-host: env var; cloud: KMS). Never logged, never in error messages.

#### `resources`

Stored similarly to credentials. Less secret-sensitive (HTTP clients, DB pools, etc.) but still scoped:

```sql
CREATE TABLE resources (
    id             BYTEA PRIMARY KEY,          -- res_ ULID
    workspace_id   BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slug           TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    kind           TEXT NOT NULL,
    config         JSONB NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    created_by     BYTEA NOT NULL,
    version        BIGINT NOT NULL DEFAULT 0,
    deleted_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_resources_workspace_slug
    ON resources (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;
```

### Layer 7 — Quotas and rate limits

#### `org_quotas`, `org_quota_usage`, `workspace_quota_usage`

Defined in spec 10:

```sql
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

CREATE TABLE workspace_quota_usage (
    workspace_id                    BYTEA PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    concurrent_executions           INT NOT NULL DEFAULT 0,
    active_workflows                INT NOT NULL DEFAULT 0,
    updated_at                      TIMESTAMPTZ NOT NULL
);

CREATE TABLE workspace_dispatch_state (
    workspace_id        BYTEA PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    last_dispatched_at  TIMESTAMPTZ NOT NULL
);
```

### Layer 8 — Audit and slug history

#### `slug_history`

```sql
CREATE TABLE slug_history (
    kind            TEXT NOT NULL,           -- 'org' / 'workspace' / 'workflow' / ...
    scope_id        BYTEA,                   -- NULL for org, else parent id
    old_slug        TEXT NOT NULL,
    resource_id     BYTEA NOT NULL,          -- target entity
    renamed_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (kind, scope_id, old_slug)
);

CREATE INDEX idx_slug_history_expiry
    ON slug_history (expires_at)
    WHERE expires_at > NOW();
```

#### `audit_log`

High-level audit events separate from `execution_journal`:

```sql
CREATE TABLE audit_log (
    id               BYTEA PRIMARY KEY,       -- ULID
    org_id           BYTEA NOT NULL,
    workspace_id     BYTEA,                   -- NULL for org-level events
    actor_kind       TEXT NOT NULL,           -- 'user' / 'service_account' / 'system'
    actor_id         BYTEA,                   -- nullable for system events
    action           TEXT NOT NULL,           -- 'workflow.created' / 'credential.rotated' / 'user.invited' / ...
    target_kind      TEXT,
    target_id        BYTEA,
    details          JSONB,
    ip_address       INET,
    user_agent       TEXT,
    emitted_at       TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_audit_log_by_org
    ON audit_log (org_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_actor
    ON audit_log (actor_kind, actor_id, emitted_at DESC);

CREATE INDEX idx_audit_log_by_action
    ON audit_log (action, emitted_at DESC);
```

**Retention:** 90 days default, configurable per plan. Enterprise may require 1 year or indefinite.

## SQLite differences

SQLite doesn't support:

- `BYTEA` → use `BLOB`
- `UUID` column type → use `BLOB`
- `JSONB` → use `TEXT` with JSON validation (sqlite has `json1` extension for querying)
- `INET` → use `TEXT`
- `TIMESTAMPTZ` → use `TEXT` with ISO 8601 format, or `INTEGER` unix timestamp
- `ARRAY` types → use `JSON` array in `TEXT`
- `WHERE expires_at > NOW()` in index predicates — SQLite allows constant expressions only, not `NOW()`

**Solution: maintain two schema files** (`sqlite.sql` and `postgres.sql`) with dialect-specific translations. Core table structure is identical, types and indexes differ.

**Or:** use abstraction tool like `sqlx` with query macros that work across backends. Chosen: **parallel schema files**, explicit and debuggable.

## Retention policy summary

| Table | Retention rule | Backing job |
|---|---|---|
| `users`, `orgs`, `workspaces` | Soft delete → 30d hard delete | daily cleanup |
| `sessions` | Delete expired daily | daily |
| `verification_tokens` | Delete consumed or expired daily | daily |
| `workflow_versions` | 90 days unless referenced | daily GC |
| `executions` (terminal) | 90 days default, plan-configurable | daily GC |
| `execution_nodes` | Cascade with executions | — |
| `execution_journal` | Cascade with executions | — |
| `execution_control_queue` | 7 days after processed | daily cleanup |
| `trigger_events` (dispatched) | 30 days | daily cleanup |
| `cron_fire_slots` | 7 days | daily cleanup |
| `pending_signals` (consumed) | 30 days | daily cleanup |
| `slug_history` | Until `expires_at` | daily cleanup |
| `audit_log` | 90 days default | daily cleanup |

## Canon §11.5 — final durability matrix

```markdown
### 11.5 Durability matrix

| Artifact | Durability | Operator meaning |
|---|---|---|
| `orgs`, `workspaces`, `users` rows | Durable | Tenant and identity truth |
| `workflows`, `workflow_versions` rows | Durable | Definition truth; executions pinned to version |
| `executions` row | Durable, CAS transitions | Authoritative run state |
| `execution_nodes` row (inc. `state` column) | Durable per checkpoint policy | Per-attempt audit + StatefulAction state |
| `execution_journal` | Durable, append-only | Replayable event timeline |
| `execution_control_queue` | Durable | At-least-once cancel/run signals (§12.2) |
| `trigger_events` | Durable with dedup (unique constraint) | At-least-once trigger ingestion |
| `cron_fire_slots` | Durable with unique constraint | Leaderless cron coordination |
| `credentials` (encrypted_secret) | Durable, encrypted at rest | Authoritative credential store |
| `audit_log` | Durable, append-only | Compliance trail |
| In-process state buffer | Ephemeral | Lost on crash, recovery via last checkpoint |
| In-process eventbus | Ephemeral | Subscribers must own their durability |
| Rate limiter state | Ephemeral per-process | By design, not authoritative |
```

## Migrations

**Greenfield v1:** apply all tables in order respecting FK dependencies.

**Migration ordering:**

1. `users`, `oauth_links`, `sessions`, `personal_access_tokens`, `verification_tokens`
2. `orgs`, `workspaces`, `org_members`, `workspace_members`, `service_accounts`
3. `workflows`, `workflow_versions` (+ FK cycle resolution)
4. `credentials`, `resources`
5. `triggers`, `trigger_events`, `cron_fire_slots`, `pending_signals`
6. `executions`, `execution_nodes`, `execution_journal`, `execution_control_queue`
7. `org_quotas`, `org_quota_usage`, `workspace_quota_usage`, `workspace_dispatch_state`
8. `slug_history`, `audit_log`

**Future migrations:** each schema change lands as a timestamped migration file (`nnnn-description.sql`). Up migrations always, down migrations optional.

## Testing criteria

- All tables create successfully on fresh SQLite database
- All tables create successfully on fresh Postgres database
- Schema identical logically between backends (types differ)
- FK integrity on cascade deletes
- Unique constraints enforce dedup
- Indexes provide expected query plans (explain output checked in tests)
- Backup/restore roundtrip preserves all data

## Performance targets

- Single-row insert to any table: **< 5 ms p99**
- Bulk insert (100 rows): **< 50 ms p99**
- Typical executions list query (workspace, last 100): **< 20 ms p99**
- Claim query from `executions` with lease: **< 10 ms p99**
- Dedup check on `trigger_events` via unique constraint: **< 5 ms p99**

## Module boundaries

| Component | Crate |
|---|---|
| SQL migration files | `crates/storage/migrations/sqlite/`, `crates/storage/migrations/postgres/` |
| Repositories (`OrgRepo`, `ExecutionRepo`, etc.) | `nebula-storage` |
| Row types (`OrgRow`, `ExecutionRow`, etc.) | `nebula-storage::rows` |
| Encoding between row types and domain types | `nebula-storage::mapping` |
| Connection pool management | `nebula-storage::pool` |

## Open questions

- **Partitioning** — for cloud with millions of executions, partition `executions` and `execution_journal` by month or by workspace. Deferred to when measured necessary.
- **Archive cold storage** — move old executions to cheap storage (S3) after 90 days. Deferred.
- **Event sourcing for executions** — alternative model where `execution_journal` is the truth and `executions` row is a materialized view. More complex, deferred.
- **Read replicas** — for cloud, route list queries to Postgres read replicas. Deferred.
- **Encrypted search over credentials metadata** — can we search credentials without decrypting? Deferred.
