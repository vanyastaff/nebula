# Spec 10 — Timeouts, budgets, quotas, rate limits

> **Status:** draft
> **Canon target:** §11.7 (new), §12.3 (quota defaults update), §5 (scope table row)
> **Depends on:** 02 (tenancy for quota scope), 08 (cancel for timeout escalation), 09 (retry for budget interaction)
> **Depended on by:** 17 (multi-process — fair scheduling)

## Problem

Four different concepts get smeared together under «rate limit» or «timeout»:

1. **Timeout** — wall-clock deadline for one operation
2. **Budget** — pool of something consumed across operations
3. **Quota** — tenant-level limit checked at admission (billing-related)
4. **Rate limit** — density protection against spikes

Airflow famously confused retry count with retry budget — exponential backoff without total_timeout ran workflows for hours. Temporal has five overlapping timeouts on activities, most users don't understand them. Stripe documentation explicitly warns against «exactly once» and «unlimited retry», naming these as anti-patterns.

## Decision

**Four distinct concepts with non-overlapping roles.** Each has a clear enforcement point, clear failure semantics, and clear scope.

## The four concepts

| Concept | What it limits | Scope | Enforced at | Failure |
|---|---|---|---|---|
| **Timeout** | wall-clock time of one thing | one object | runtime wrapper | task fails, maybe retry |
| **Budget** | aggregate resource over operations | series of ops | each consumption | op fails |
| **Quota** | tenant-level limit (usually monthly or absolute) | per-tenant | admission (API handler) | `429` / `402` |
| **Rate limit** | request density against spikes | per-identity-key | API middleware | `429` + Retry-After |

## 10.1 — Timeouts (three levels)

### Taxonomy

| Level | Duration default | Override source | On expiration |
|---|---|---|---|
| **Node attempt** | 5 minutes | `ActionMetadata::default_attempt_timeout` or `NodeDefinition::timeout_override` | `Err(Timeout)`, may retry per R2 |
| **Retry total** | 30 minutes | `ActionMetadata::retry_policy.total_timeout` | No more retries, final `Failed(RetryBudgetExhausted)` |
| **Execution wall-clock** | 24 hours | `WorkflowDefinition::execution_timeout` or API start-time override | Cancel cascade (graceful, not terminate) |

**Plus for StatefulAction (spec 14):**

| Level | Duration default | Override source | On expiration |
|---|---|---|---|
| **Step timeout** | 5 minutes | same as node attempt | Step fails, re-runs from last checkpoint |
| **Stateful max duration** | 7 days | `ActionMetadata::stateful_max_duration` | `Failed(StatefulTimeout)` |

### Waterfall rule

```
node_attempt_timeout ≤ retry_total_timeout ≤ execution_timeout
```

Enforced by workflow validator at save time:

```rust
pub fn validate_timeout_waterfall(
    action_meta: &ActionMetadata,
    node: &NodeDefinition,
    workflow: &WorkflowDefinition,
) -> Result<(), ValidationError> {
    let attempt = node.timeout_override
        .unwrap_or(action_meta.default_attempt_timeout);
    let retry_total = action_meta.retry_policy.total_timeout
        .unwrap_or(Duration::MAX);
    let execution = workflow.execution_timeout;
    
    if attempt > retry_total {
        return Err(ValidationError::TimeoutWaterfall {
            msg: format!("attempt_timeout ({}s) > retry_total_timeout ({}s)",
                         attempt.as_secs(), retry_total.as_secs()),
        });
    }
    if retry_total > execution {
        return Err(ValidationError::TimeoutWaterfall {
            msg: format!("retry_total_timeout > execution_timeout"),
        });
    }
    Ok(())
}
```

**Why this rule:** if `retry_total > execution`, retries fire after execution already expired — wasted work. If `attempt > retry_total`, retry policy never gets a chance. Waterfall keeps semantics clean.

### Enforcement points

**Node attempt timeout:**

```rust
// runtime wrapper
async fn run_with_timeout(
    action_future: impl Future<Output = Result<Value, ActionError>>,
    timeout: Duration,
) -> Result<Value, ActionError> {
    match tokio::time::timeout(timeout, action_future).await {
        Ok(result) => result,
        Err(_) => Err(ActionError::Timeout),
    }
}
```

**Retry total timeout:**

Enforced in retry decision logic (spec 09):

```rust
if started_at.elapsed() >= policy.total_timeout {
    return RetryDecision::ExhaustedBudget;
}
```

**Execution wall-clock:**

Background scanner or engine-level deadline check:

```sql
-- Scheduler query for expired executions
UPDATE executions
SET status = 'Cancelling',
    cancel_reason = 'execution_timeout',
    version = version + 1
WHERE status = 'Running'
  AND started_at + (execution_timeout::INTERVAL) < NOW();
```

Executions past their deadline are cancelled via the normal cancel cascade (spec 08), not hard-killed. Gives in-flight actions a chance to clean up.

## 10.2 — Budgets

Only one persistent budget in v1: `retry_policy.total_timeout`. It's the pool of time consumed across retry attempts (accounting = `now - first_attempt_start`).

**Explicit non-goals for v1:**

- **Compute-time budget per action** — measuring CPU time per future is expensive and inaccurate in Rust async (no `getrusage` per task). Deferred.
- **Memory budget per action** — `jemalloc` profiling per future is expensive. Deferred.
- **Network bandwidth budget per execution** — requires in-process I/O tracking, deferred.
- **Disk budget per execution** — same. Deferred.

If operational experience later shows need, add specific budgets as new concepts, not as generic «resource budget».

## 10.3 — Quotas

Quotas are **tenant-level limits** checked at admission, before any work starts. Atomic compare-and-increment to avoid races.

### Quota types

| Quota | Scope | Period | Self-host default | Cloud default | Billing |
|---|---|---|---|---|---|
| **Concurrent executions** | per workspace | instant | 100 | free 20, team 100, enterprise 500 | no |
| **Concurrent executions** | per org | instant | 500 | free 50, team 500, enterprise 5000 | no |
| **Executions per month** | per org | monthly | unlimited | free 10k, team 100k, enterprise unlim | **yes** |
| **Active workflows** | per workspace | instant | unlimited | free 5, team 50, enterprise unlim | **yes** |
| **Total workflows** | per org | instant | unlimited | free 10, team 200, enterprise unlim | **yes** |
| **Storage (journal + state + nodes)** | per org | instant | unlimited | free 100 MB, team 10 GB, enterprise 1 TB | **yes** |
| **Org members** | per org | instant | 1000 | free 3, team 50, enterprise unlim | **yes** |
| **Workspaces** | per org | instant | 1000 | free 2, team 20, enterprise unlim | **yes** |
| **Service accounts** | per org | instant | 100 | free 1, team 20, enterprise 500 | **yes** |
| **Trigger events inbox depth** | per workspace | instant | 10000 | free 1000, team 10000, enterprise 100000 | no |
| **PAT count** | per principal | instant | 50 | same | no |

**Billing column:** quotas that correspond to plan tiers. Exceeding triggers upgrade prompt. Non-billing quotas are operational safety limits (prevent runaway workflows from OOM-ing the system).

### SQL enforcement pattern

**Atomic compare-and-increment** — no check-then-set race:

```sql
-- Starting an execution checks concurrent quota
UPDATE org_quota_usage
SET concurrent_executions = concurrent_executions + 1,
    updated_at = NOW()
WHERE org_id = $1
  AND concurrent_executions + 1 <= (
      SELECT concurrent_executions_limit 
      FROM org_quotas 
      WHERE org_id = $1
  )
RETURNING concurrent_executions;
```

- **If UPDATE affected 1 row:** within quota, execution proceeds
- **If UPDATE affected 0 rows:** at limit, return `429 Too Many Requests` with `Retry-After: <estimate>` header

### Release

Quota usage decremented when execution reaches terminal state, in the same transaction as status transition:

```sql
BEGIN;
UPDATE executions
SET status = 'Succeeded', finished_at = NOW(), version = version + 1
WHERE id = $1 AND version = $2;

UPDATE org_quota_usage
SET concurrent_executions = concurrent_executions - 1,
    updated_at = NOW()
WHERE org_id = $3;
COMMIT;
```

**Critical:** release must be atomic with state transition, otherwise crashed workers leak quota slots forever. If transaction fails, neither happens.

### Crash recovery for leaked quota slots

Belt and braces — periodic reconciliation job:

```sql
-- Runs every 15 minutes
UPDATE org_quota_usage q
SET concurrent_executions = (
    SELECT COUNT(*) FROM executions 
    WHERE org_id = q.org_id
      AND status = 'Running'
)
WHERE updated_at < NOW() - INTERVAL '15 minutes';
```

Ensures counter is eventually consistent with actual row count, fixing drift from crashed workers or bugs.

### Monthly quota reset

Background job on first day of month:

```sql
UPDATE org_quota_usage
SET executions_this_month = 0,
    month_reset_at = NOW();
```

Resets counters atomically. Quotas that are per-month (executions) are based on this reset.

### Storage quota

**Soft limit with warning + hard limit that blocks writes:**

Storage usage computed periodically (not on every write, too expensive):

```sql
-- Background job every 5 minutes per org
UPDATE org_quota_usage
SET storage_bytes = (
    SELECT
        COALESCE(SUM(pg_column_size(input) + pg_column_size(output) + pg_column_size(state)), 0)
    FROM execution_nodes WHERE org_id = q.org_id
) + (
    SELECT COALESCE(SUM(pg_column_size(payload)), 0)
    FROM execution_journal WHERE org_id = q.org_id
);
```

**Soft limit = plan limit:** warning shown in UI, new executions still accepted.

**Hard limit = soft × 1.2:** new executions rejected with `507 Insufficient Storage`, existing continue.

**Cleanup triggered eagerly when soft limit hit:** retention job moves execution history past retention period to cold storage (or deletes, depending on plan).

## 10.4 — Rate limits

Rate limit is **spike protection**, not quota. Same tenant can hit rate limit without using up their monthly quota.

### What gets rate-limited

| Limit | Scope | Default |
|---|---|---|
| API calls per PAT | per PAT identity | 100/sec (rolling) |
| API calls per session | per session | 50/sec |
| Login attempts per IP | per IP | 5/min |
| Login attempts per user | per user | 5/min |
| Password reset per email | per email | 3/hour |
| Signup per IP | per IP | 10/hour |
| Execution starts per workspace | per workspace | 50/sec |
| Webhook events per trigger | per trigger | 100/sec (configurable per trigger) |

**Keyed by identity, not by tenant.** One user's rate limit doesn't affect another user in the same org.

### Implementation

`governor` crate — zero-alloc, lock-free token bucket:

```rust
// nebula-api/src/middleware/rate_limit.rs
use governor::{Quota, RateLimiter, clock::DefaultClock};
use std::sync::Arc;

pub struct ApiRateLimiter {
    // Keyed by PatId or session_id
    per_pat: Arc<RateLimiter<PatId, _, DefaultClock>>,
    per_session: Arc<RateLimiter<SessionId, _, DefaultClock>>,
    per_ip_login: Arc<RateLimiter<IpAddr, _, DefaultClock>>,
    // ...
}

impl ApiRateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            per_pat: Arc::new(RateLimiter::keyed(
                Quota::per_second(nonzero!(100u32))
                    .allow_burst(nonzero!(200u32))
            )),
            // ... etc
        }
    }

    pub fn check_pat(&self, pat: PatId) -> Result<(), RateLimitError> {
        self.per_pat.check_key(&pat).map_err(|neg| RateLimitError {
            retry_after: neg.wait_time_from(DefaultClock::default().now()),
        })
    }
}
```

### In-memory per-process in v1

Rate limit state is **in-memory**, not durable. Each worker process has its own limiter. For single-process v1, this is correct.

**Multi-process consideration (v2):** rate limits become per-process, so N workers → effective rate limit is N × configured. Options:

1. **Divide by replica count** — config-tuned, fragile
2. **Shared Redis** — backend-agnostic `governor` or manual token bucket in Redis
3. **Accept per-process** — document, tell operators «configured for single process»

**v1 decision: accept per-process, document clearly.** Redis backend deferred to v2.

### 429 response format

```http
HTTP/1.1 429 Too Many Requests
Content-Type: application/problem+json
Retry-After: 5

{
    "type": "https://nebula.io/errors/rate-limited",
    "title": "Rate limit exceeded",
    "status": 429,
    "detail": "Too many requests. Try again in 5 seconds.",
    "error_code": "RATE_LIMITED",
    "limit_category": "per_pat",
    "retry_after_seconds": 5
}
```

`Retry-After` header is mandatory on 429 responses — standard HTTP practice, client libraries honor it automatically.

### IP extraction for rate limiting

Behind reverse proxy, `socket.peer_addr()` is the proxy's IP — all traffic rate-limited together if we use it.

**Solution:** read `X-Forwarded-For` header, but **only if request came from trusted proxy**.

```toml
[api.trusted_proxies]
cidrs = ["10.0.0.0/8", "172.16.0.0/12"]
```

If source IP is in `trusted_proxies`, use the leftmost entry in `X-Forwarded-For`. Otherwise, use `peer_addr`. This prevents header spoofing from untrusted sources while allowing legitimate proxy deployments.

## 10.5 — Fair scheduling between workspaces

Without fair scheduling, one busy workspace can starve others. Naive FIFO on `trigger_events` or `executions` produces Airflow-style starvation (one DAG with 1000 tasks blocks all others).

### Weighted round-robin

Each workspace tracks «last dispatched» timestamp:

```sql
CREATE TABLE workspace_dispatch_state (
    workspace_id        BYTEA PRIMARY KEY,
    last_dispatched_at  TIMESTAMPTZ NOT NULL
);
```

Dispatcher query orders by least-recently-dispatched workspace first:

```sql
SELECT * FROM executions
WHERE status IN ('Pending', 'Queued')
  AND (claimed_until IS NULL OR claimed_until < NOW())
ORDER BY
  (SELECT COALESCE(last_dispatched_at, '1970-01-01')
   FROM workspace_dispatch_state
   WHERE workspace_id = executions.workspace_id) ASC NULLS FIRST,
  executions.created_at ASC
LIMIT 10
FOR UPDATE SKIP LOCKED;
```

After claiming, update `last_dispatched_at`:

```sql
INSERT INTO workspace_dispatch_state (workspace_id, last_dispatched_at)
VALUES ($1, NOW())
ON CONFLICT (workspace_id) DO UPDATE
SET last_dispatched_at = NOW();
```

**Property:** if workspace A has 1000 pending executions and workspace B has 10, dispatcher alternates between them, giving B a chance every other iteration.

**Limitation:** not a formal fair scheduler with guarantees. Within a workspace, FIFO. Between workspaces, LRU. Edge cases where two workspaces have very different burst patterns can still see imperfect sharing.

**Priority override (v2, `planned`):** workspace has `priority` field, scheduler weights by it. For v1, all workspaces equal priority.

## 10.6 — Enforcement points cheat sheet

```
Incoming API request
  ↓
┌───────────────────────────────────────┐
│ Rate limit middleware                  │  — in-memory bucket, 429 on exceed
│ (nebula-api, before auth)              │
└───────────────────────────────────────┘
  ↓
┌───────────────────────────────────────┐
│ Auth middleware                        │
│ (nebula-api)                           │
└───────────────────────────────────────┘
  ↓
┌───────────────────────────────────────┐
│ Tenancy middleware                     │
│ (nebula-api)                           │
└───────────────────────────────────────┘
  ↓
┌───────────────────────────────────────┐
│ RBAC middleware                        │
│ (nebula-api)                           │
└───────────────────────────────────────┘
  ↓
┌───────────────────────────────────────┐
│ Handler                                │
│ (e.g., start_execution)                │
│  ↓                                     │
│  Quota check (atomic CAS)              │  — 429 on concurrent, 402 on plan
│  ↓                                     │
│  Enqueue work                          │
└───────────────────────────────────────┘
  ↓
Scheduler picks up (with fair ordering)
  ↓
Dispatcher claims, starts execution
  ↓
┌───────────────────────────────────────┐
│ Runtime wraps action                   │
│  Attempt timeout (tokio::time::timeout)│
│  Cancel cascade (spec 08)              │
│  Retry budget check (spec 09)          │
└───────────────────────────────────────┘
  ↓
On completion: atomic quota release + state transition
```

**Each concept has exactly one enforcement point.** No duplicated checks, no drift.

## Data model

### `org_quotas` and `org_quota_usage`

```sql
CREATE TABLE org_quotas (
    org_id                          BYTEA PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    plan                            TEXT NOT NULL,
    
    -- Concurrent limits
    concurrent_executions_limit     INT NOT NULL DEFAULT 50,
    
    -- Monthly limits
    executions_per_month_limit      BIGINT,  -- NULL = unlimited
    
    -- Instant limits
    active_workflows_limit          INT,
    total_workflows_limit           INT,
    workspaces_limit                INT,
    org_members_limit               INT,
    service_accounts_limit          INT,
    
    -- Storage
    storage_bytes_limit             BIGINT,
    
    updated_at                      TIMESTAMPTZ NOT NULL
);

CREATE TABLE org_quota_usage (
    org_id                          BYTEA PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    
    -- Current counters (decremented on release)
    concurrent_executions           INT NOT NULL DEFAULT 0,
    active_workflows                INT NOT NULL DEFAULT 0,
    total_workflows                 INT NOT NULL DEFAULT 0,
    workspaces                      INT NOT NULL DEFAULT 0,
    org_members                     INT NOT NULL DEFAULT 0,
    service_accounts                INT NOT NULL DEFAULT 0,
    storage_bytes                   BIGINT NOT NULL DEFAULT 0,
    
    -- Monthly counters (reset on first of month)
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
```

**Partitioning note:** if `org_quota_usage` becomes a hot row for a busy org (contention on `concurrent_executions` counter), sharding by org is straightforward since the row is keyed by `org_id`.

## Configuration surface

```toml
[quotas.self_host]
# Generous defaults for self-host (effectively unlimited)
concurrent_executions_per_workspace = 100
concurrent_executions_per_org = 500
# Most other quotas disabled via NULL in DB

[quotas.cloud.free]
concurrent_executions_per_workspace = 20
concurrent_executions_per_org = 50
executions_per_month = 10_000
active_workflows_per_workspace = 5
total_workflows_per_org = 10
workspaces_per_org = 2
org_members = 3
service_accounts = 1
storage_bytes = 104_857_600  # 100 MB

[quotas.cloud.team]
concurrent_executions_per_workspace = 100
# ... etc

[rate_limits]
api_per_pat_per_second = 100
api_per_session_per_second = 50
login_per_ip_per_minute = 5
login_per_user_per_minute = 5
signup_per_ip_per_hour = 10
password_reset_per_email_per_hour = 3
execution_start_per_workspace_per_second = 50

[rate_limits.trusted_proxies]
cidrs = []  # e.g., ["10.0.0.0/8"] for cloud behind Cloudflare

[timeouts]
default_node_attempt = "5m"
default_retry_total = "30m"
default_execution_wall_clock = "24h"
default_stateful_max_duration = "7d"
execution_timeout_scanner_interval = "30s"
```

## Flows

### Start execution flow

```
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions
  ↓
Rate limit middleware checks:
  - per_session (if cookie) or per_pat (if Bearer)
  - per_workspace execution_starts
  429 on exceed
  ↓
Auth → Tenancy → RBAC (standard chain)
  ctx.require(Permission::WorkflowExecute)
  ↓
Handler start_execution:
  1. Load workflow, check active (not archived)
  2. Atomic quota check:
     UPDATE org_quota_usage
       SET concurrent_executions = concurrent_executions + 1
       WHERE org_id = ? AND concurrent_executions + 1 <= concurrent_executions_limit
     If 0 rows → 429 "concurrent_executions_exceeded"
  3. Check monthly quota (if cloud):
     UPDATE org_quota_usage
       SET executions_this_month = executions_this_month + 1
       WHERE org_id = ? AND (executions_this_month + 1 <= executions_per_month_limit OR executions_per_month_limit IS NULL)
     If 0 rows → 402 "monthly_quota_exceeded"
  4. INSERT executions (status='Pending', input, workflow_version_id, ...)
  5. Return 202 Accepted { "execution_id": "exec_01..." }
```

### Execution terminal flow

```
Engine finishes execution, calls transition_to_terminal:
  ↓
BEGIN TRANSACTION
  UPDATE executions SET status = 'Succeeded', finished_at = NOW(), version = version + 1
    WHERE id = ? AND version = ?
  UPDATE org_quota_usage SET concurrent_executions = concurrent_executions - 1
    WHERE org_id = ?
  UPDATE workspace_quota_usage SET concurrent_executions = concurrent_executions - 1
    WHERE workspace_id = ?
COMMIT
  ↓
nebula-eventbus: ExecutionCompleted
```

### Quota reconciliation (belt and braces)

```
Background job every 15 minutes:
  FOR each org:
    actual = SELECT COUNT(*) FROM executions WHERE org_id = ? AND status = 'Running'
    current = SELECT concurrent_executions FROM org_quota_usage WHERE org_id = ?
    IF actual != current:
      log warning
      UPDATE org_quota_usage SET concurrent_executions = actual
      emit metric nebula_quota_drift_corrected_total
```

## Edge cases

**Quota decrement on crashed worker:** worker dies with execution in Running state, lease expires, another worker claims. Counter stays same (not double-counted). When execution eventually completes or is cancelled, counter decrements. Reconciliation job catches any permanent drift.

**Monthly quota rolled over mid-execution:** executions in-flight at month boundary don't re-count against new month. `executions_this_month` counts **started** executions, not active. Starting is the billing event.

**Clock skew between workers in multi-process:** rate limiters use monotonic time, not wall clock. Safe.

**User hits concurrent quota during burst then immediately retries:** legitimate traffic gets 429. They receive `Retry-After`, client backoff. No special handling needed beyond good client libraries.

**Plan upgrade during active execution:** `org_quotas` updated atomically. New quotas apply to new executions. In-flight continue uninterrupted (they already hold their slot).

**Plan downgrade:** tricky. If customer on team plan has 100 active workflows, downgrades to free (limit 5), what happens? Policy decision:

- **Option A:** downgrade blocked until customer reduces usage manually. Gentler but requires UX.
- **Option B:** downgrade succeeds, existing items grandfathered (can't create new, existing keep running). Airflow-style.
- **Option C:** downgrade fails with explicit error listing blocking resources.

**Recommendation: B.** Grandfather existing, block new. Most user-friendly without surprise deletion.

**Webhook flood from one trigger:** per-trigger rate limit protects against one misbehaving sender. Doesn't affect other triggers.

**Brute force on single user account:** per-user rate limit triggers before per-IP (if attacker rotates IPs). Both layers active.

## Testing criteria

**Unit tests:**
- `BackoffStrategy::delay_for` (from spec 09 — same math)
- Timeout waterfall validator rejects invalid configurations
- Rate limiter token bucket refill arithmetic

**Integration tests:**
- Concurrent execution quota: start 101 executions on workspace with limit 100, last one returns 429
- Monthly quota: start 10_001 executions in a month on free plan, last returns 402
- Quota decrement on success / failure / cancel (all three paths)
- Rate limit on login endpoint: 6th attempt within 60s returns 429
- Rate limit per PAT: 101st request within 1s returns 429
- Execution timeout: start execution with 5s timeout, action sleeps 10s, execution cancelled
- Retry total timeout: action fails repeatedly, total elapsed exceeds total_timeout, final Failed without more retries
- Fair scheduling: two workspaces with 100 and 10 pending each, dispatcher alternates (measured via claim order)
- Quota reconciliation: manually deflate counter, reconciliation job corrects after 15 minutes

**Load tests:**
- 10_000 API requests at rate limit boundary — 429 applied correctly
- 1_000 concurrent execution starts with atomic quota check — no drift

**Security tests:**
- Rate limit bypass via header spoofing (X-Forwarded-For without trusted_proxies)
- Concurrent quota race via burst of starts

**Property tests:**
- Quota counter never goes negative
- Counter after reconciliation equals actual count
- Rate limiter: requests within budget always allowed, requests over budget always denied

## Performance targets

- Quota check (atomic UPDATE): **< 5 ms p99**
- Rate limit check (in-memory): **< 10 µs p99**
- Fair scheduling query: **< 20 ms p99** for up to 10_000 pending rows
- Execution timeout scanner: **< 100 ms** per scan for 100_000 active executions
- Quota reconciliation job: **< 5 seconds** per org for large orgs

## Module boundaries

| Component | Crate |
|---|---|
| `RateLimiter`, `governor` integration | `nebula-api` (middleware) |
| Rate limit config types | `nebula-core` or `nebula-config` |
| Quota check SQL | `nebula-storage` |
| Quota reconciliation job | `nebula-engine` (background task) |
| Timeout enforcement (runtime wrapper) | `nebula-runtime` |
| Execution timeout scanner | `nebula-engine` |
| Fair scheduling query | `nebula-storage` + `nebula-engine` |

## Migration path

**Greenfield** — no prior quotas.

**Rollout discipline:** when adding a new quota, default to generous for existing customers to avoid breaking them. New plans can have stricter defaults.

## Open questions

- **Dedicated quota service** — if org becomes a hot row, spin off a separate service (with Redis or similar fast backend)? v2 consideration when scale demands.
- **Priority workspaces** — v2 feature, adds `priority` column, weights fair scheduling.
- **Reserved capacity** — enterprise customers might want «always 500 concurrent slots available» guarantee. Different from simple quota — requires resource reservation. Deferred.
- **Grace period on quota over-limit** — allow soft burst above limit for short periods? Probably no — clean rejection is easier to reason about.
- **Cost-based budget (dollars)** — integrate with billing to say «stop spending more than $X per day on this workflow». Interesting but deferred until billing infrastructure exists.
