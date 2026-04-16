# Spec 09 — Retry cascade (four-layer model)

> **Status:** draft — closes §11.2 «false capability»
> **Canon target:** §11.2 (rewrite), §14 anti-pattern cross-ref
> **Depends on:** 08 (cancel wins over retry), 16 (storage)
> **Depended on by:** 14 (stateful — per-iteration idempotency), 11 (triggers — retry on dispatch)

## Problem

Canon §11.2 currently says engine-level retry is `planned` / `false capability` — types exist but runtime doesn't honor them. This spec defines the full retry model to close that debt.

«Retry» is four different things that get smeared together:

1. **In-call retry** — wrapping an outbound HTTP call in a retry loop inside action code
2. **Action-level retry** — re-running the whole action when it fails transiently
3. **Workflow edge routing** — DAG edge that catches error and routes to compensation node
4. **Manual re-run** — operator button that creates a fresh execution from failed one

Each has different semantics, different persistence needs, different author ergonomics. A single «retry policy» blob that tries to cover all of them becomes incoherent.

## Decision

**Four-layer retry cascade, each layer with its own scope and mechanism, each tries to recover, unrecovered failure propagates to the next layer up.**

- **R1** — in-action retry via `nebula-resilience` pipeline (in-memory, author-controlled)
- **R2** — engine-level attempt retry via `ActionMetadata::retry_policy` + `Classify::retryable()` (persisted, runtime-controlled)
- **R3** — DAG `on_error` edges (workflow definition, no cycles)
- **R4** — manual workflow restart (operator action, fresh execution)

Cancel always wins over retry. Idempotency key is stable per attempt, not per retry.

## Cascade diagram

```
                             ┌──────────────────────────────┐
                             │ R4. Manual workflow restart   │
                             │    Operator: "Restart run"    │
                             │    Creates new execution      │
                             │    (fresh exec_id + attempts) │
                             └────────────▲─────────────────┘
                                          │ Nothing recovered,
                                          │ operator intervenes
                             ┌────────────┴─────────────────┐
                             │ R3. DAG on_error edge         │
                             │    Workflow author defined    │
                             │    node_a --on_error--> node_b│
                             │    Routes to compensation /   │
                             │    fallback / notification    │
                             └────────────▲─────────────────┘
                                          │ Node failed after all
                                          │ R2 attempts, has edge?
                             ┌────────────┴─────────────────┐
                             │ R2. Engine attempt retry      │
                             │    ActionMetadata::retry_policy│
                             │    + Classify::retryable()    │
                             │    Persisted in execution_nodes│
                             │    New attempt = new row      │
                             └────────────▲─────────────────┘
                                          │ Action returned Err,
                                          │ Classify says retryable
                             ┌────────────┴─────────────────┐
                             │ R1. In-action retry           │
                             │    nebula-resilience pipeline │
                             │    Wraps outbound call        │
                             │    In-memory, fast (ms)       │
                             └──────────────────────────────┘
```

## R1 — in-action retry via `nebula-resilience`

Author wraps specific operations in retry policy, inside action code:

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let response = ctx.resilience
        .policy()
        .retry(RetryConfig::exponential_jitter(
            max_attempts: 5,
            initial: Duration::from_millis(100),
            max: Duration::from_secs(2),
        ))
        .timeout(Duration::from_secs(10))
        .execute(|| async {
            client.post(&input.url).send().await
        })
        .await
        .map_err(|e| ActionError::from_classified(e))?;

    Ok(Output::from(response))
}
```

**Characteristics:**

- **In-memory** — no persistence
- **Fast** — millisecond budgets typical
- **Author-owned** — author decides where to wrap
- **Not visible to runtime** — looks like a single action call from outside
- **Good for** — transient network blips, rate limits (429), 500/503 from flaky upstream

If `nebula-resilience` exhausts its attempts, action returns `Err(_)` to runtime. Runtime then decides whether R2 applies.

**Status:** `implemented` — `nebula-resilience` crate exists.

## R2 — engine-level attempt retry

This is the layer that closes `§11.2` false capability.

### `ActionMetadata::retry_policy`

```rust
// nebula-action/src/metadata.rs

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of attempts total (first attempt counts).
    /// 1 = no retry, 3 = initial + 2 retries.
    pub max_attempts: u32,

    /// Backoff strategy between attempts.
    pub backoff: BackoffStrategy,

    /// Which errors trigger retry. Default: ClassifyBased.
    pub retry_on: RetryClassifier,

    /// Hard ceiling on time across all attempts.
    /// Prevents exponential backoff from turning into days of waiting.
    pub total_timeout: Option<Duration>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: BackoffStrategy::ExponentialJitter {
                initial: Duration::from_secs(1),
                factor: 2.0,
                max: Duration::from_secs(60),
            },
            retry_on: RetryClassifier::ClassifyBased,
            total_timeout: Some(Duration::from_secs(600)), // 10 minutes
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackoffStrategy {
    /// Fixed delay between attempts.
    Fixed(Duration),
    /// Linear: delay = initial + step * attempt_number.
    Linear { initial: Duration, step: Duration },
    /// Exponential: delay = initial * factor^attempt_number, capped at max.
    Exponential { initial: Duration, factor: f64, max: Duration },
    /// Exponential with full jitter: delay = rand(0, initial * factor^attempt).
    ExponentialJitter { initial: Duration, factor: f64, max: Duration },
}

impl BackoffStrategy {
    pub fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed(d) => *d,
            Self::Linear { initial, step } => *initial + *step * attempt,
            Self::Exponential { initial, factor, max } => {
                let computed = initial.as_secs_f64() * factor.powi(attempt as i32);
                Duration::from_secs_f64(computed.min(max.as_secs_f64()))
            }
            Self::ExponentialJitter { initial, factor, max } => {
                let computed = initial.as_secs_f64() * factor.powi(attempt as i32);
                let capped = computed.min(max.as_secs_f64());
                Duration::from_secs_f64(fastrand::f64() * capped)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum RetryClassifier {
    /// Retry only `ActionError::Transient` variants.
    TransientOnly,
    /// Use `Classify::retryable()` from nebula-error — most flexible.
    ClassifyBased,
    /// Never retry, regardless of error type.
    Never,
    /// Always retry until max_attempts (dangerous — use only when author knows what they're doing).
    Always,
}
```

### `ActionError` variants

```rust
// nebula-action/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// Transient failure — retry would likely succeed.
    /// Network blip, rate limit, upstream 5xx, etc.
    #[error("transient: {0}")]
    Transient(String),

    /// Transient with explicit retry hint from remote system.
    /// Use for 429 Retry-After, 503 Retry-After, etc.
    #[error("transient (retry after {retry_after:?}): {message}")]
    TransientWithHint { message: String, retry_after: Duration },

    /// Permanent failure — retry will not help.
    /// Bad input, auth failed, validation error, 400, 401, 403, 404.
    #[error("permanent: {0}")]
    Permanent(String),

    /// User or system cancelled — never retry.
    #[error("cancelled")]
    Cancelled,

    /// Escalated (hard kill) — in-flight state may be inconsistent.
    #[error("cancelled (escalated)")]
    CancelledEscalated,

    /// Internal action bug — panic or logic error.
    /// Never retried automatically; operator intervention required.
    #[error("fatal: {0}")]
    Fatal(String),

    /// Timeout — attempt timeout exceeded.
    /// Retryable if classifier allows.
    #[error("timeout")]
    Timeout,
}

impl ActionError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transient(_) | Self::TransientWithHint { .. } | Self::Timeout => true,
            Self::Permanent(_) | Self::Cancelled | Self::CancelledEscalated | Self::Fatal(_) => false,
        }
    }

    pub fn retry_hint(&self) -> Option<Duration> {
        match self {
            Self::TransientWithHint { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }
}
```

### Integration with `Classify` trait

For errors from third-party libraries that don't know about `ActionError`:

```rust
// nebula-error::Classify already exists
pub trait Classify {
    fn category(&self) -> ErrorCategory;
    fn severity(&self) -> ErrorSeverity;
    fn retryable(&self) -> bool;
    fn retry_hint(&self) -> Option<RetryHint>;
}

// Conversion for common library errors
impl From<reqwest::Error> for ActionError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            return ActionError::Timeout;
        }
        if let Some(status) = e.status() {
            match status.as_u16() {
                429 => {
                    // Check for Retry-After in headers (not always available from reqwest::Error)
                    ActionError::Transient(format!("rate limited: {}", e))
                }
                500..=599 => ActionError::Transient(e.to_string()),
                400..=499 => ActionError::Permanent(e.to_string()),
                _ => ActionError::Transient(e.to_string()),
            }
        } else if e.is_connect() || e.is_request() {
            ActionError::Transient(e.to_string())
        } else {
            ActionError::Permanent(e.to_string())
        }
    }
}
```

### Classifier priority

Three sources of classification, priority order:

1. **Explicit `retry_policy.retry_on` override** — if author set `Never` or `Always`, that wins
2. **`ActionError` variant** — author chose `Transient` vs `Permanent` when constructing the error
3. **`Classify::retryable()` on source** — library-provided classification

```rust
fn should_retry(
    err: &ActionError,
    policy: &RetryPolicy,
    attempt: u32,
    started_at: Instant,
) -> RetryDecision {
    // 1. Attempt count
    if attempt >= policy.max_attempts {
        return RetryDecision::ExhaustedAttempts;
    }
    
    // 2. Total timeout
    if let Some(total) = policy.total_timeout {
        if started_at.elapsed() >= total {
            return RetryDecision::ExhaustedBudget;
        }
    }
    
    // 3. Classifier
    let retryable = match policy.retry_on {
        RetryClassifier::TransientOnly => matches!(err, ActionError::Transient(_) | ActionError::TransientWithHint { .. }),
        RetryClassifier::ClassifyBased => err.is_retryable(),
        RetryClassifier::Never => false,
        RetryClassifier::Always => !matches!(err, ActionError::Cancelled | ActionError::CancelledEscalated | ActionError::Fatal(_)),
    };
    
    if !retryable {
        return RetryDecision::NotRetryable;
    }
    
    // 4. Compute delay
    let delay = err.retry_hint()  // remote-hinted delay takes priority
        .unwrap_or_else(|| policy.backoff.delay_for(attempt));
    
    RetryDecision::RetryAfter(delay)
}

pub enum RetryDecision {
    RetryAfter(Duration),
    ExhaustedAttempts,
    ExhaustedBudget,
    NotRetryable,
}
```

### Persistence — new attempt per retry

Each retry creates a **new row** in `execution_nodes`, with incremented `attempt` counter. Original failed attempt stays for audit.

```sql
-- After first attempt fails:
INSERT INTO execution_nodes (
    id, execution_id, logical_node_id, attempt, status, ...
) VALUES (
    'node_01...', 'exec_01...', 'charge_customer', 1, 'Failed', ...
);

-- Runtime decides to retry, schedules attempt 2:
INSERT INTO execution_nodes (
    id, execution_id, logical_node_id, attempt, status, wake_at, ...
) VALUES (
    'node_02...', 'exec_01...', 'charge_customer', 2, 'PendingRetry',
    NOW() + INTERVAL '1 second', ...
);
```

**Unique constraint:** `UNIQUE (execution_id, logical_node_id, attempt)` prevents duplicate attempts due to races.

**Idempotency key per attempt:** `{execution_id}:{logical_node_id}:{attempt}` — stable within an attempt, different between attempts. External system sees a new key for each retry, can choose to dedup at its level if it wants.

### Retry scheduling

Runtime stores `wake_at` on `PendingRetry` row. Scheduler (part of dispatcher loop, spec 17) scans:

```sql
SELECT * FROM execution_nodes
WHERE status = 'PendingRetry'
  AND wake_at <= NOW()
ORDER BY wake_at
LIMIT 10
FOR UPDATE SKIP LOCKED;
```

When row is claimed by a worker, transition to `Running`, execute action, handle result (success or further retry).

### Flow

```
Attempt 1 starts:
  INSERT execution_nodes (attempt=1, status=Running, idempotency_key=exec:node:1)
  action.execute() runs
  action returns Err(ActionError::Transient("timeout"))
  ↓
Runtime evaluates retry_policy:
  - attempt (1) < max_attempts (3) ✓
  - total_timeout not exceeded ✓
  - err.is_retryable() == true ✓
  - delay = backoff.delay_for(1) = 2s
  ↓
BEGIN TRANSACTION
  UPDATE execution_nodes SET status='Failed', finished_at=NOW(), error_kind='Transient', ...
    WHERE id='node_01...'
  INSERT INTO execution_nodes (attempt=2, status='PendingRetry', wake_at=NOW()+2s, idempotency_key=exec:node:2)
COMMIT
  ↓
Scheduler sees PendingRetry row at wake_at
  Claims, transitions to Running, runs action
  Attempt 2 uses new idempotency key exec:node:2
  ↓
External system (Stripe, etc.):
  - Sees new Idempotency-Key header: exec:node:2
  - Different from exec:node:1
  - Performs the operation fresh (or its own dedup logic applies)
```

### Stripe-like idempotency key handling

Action author chooses to pass a **stable business key**, not the Nebula-provided one, for critical operations:

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // Nebula's attempt-level key (different per retry)
    let nebula_key = ctx.idempotency_key();
    
    // Business-level key (stable across retries)
    // e.g., combined with workflow run id so retries of same run dedup,
    // but different runs don't
    let business_key = format!(
        "{}:charge:{}",
        ctx.execution_id(),
        input.customer_id
    );
    
    let charge = stripe_client.charges()
        .create(&stripe::CreateCharge {
            amount: input.amount_cents,
            customer: input.customer_id,
            idempotency_key: Some(business_key),  // author's choice
            ..Default::default()
        })
        .await
        .map_err(ActionError::from)?;
    
    Ok(Output { charge_id: charge.id })
}
```

Author can use either key based on whether they want dedup per-attempt (change on retry) or per-run (same across retries). Runtime provides the mechanism, author makes the choice per action. Spec 15 (delivery semantics) documents this as two-sided contract.

## R3 — DAG on_error edges

Workflow author adds edges to the DAG that route on failure to a different node:

```yaml
# workflow definition (conceptual)
nodes:
  charge_customer:
    action: stripe.create_charge
    input: ...
  send_receipt:
    action: email.send
    input: ...
  notify_finance_failed:
    action: slack.send_message
    input: ...
  rollback_order:
    action: orders.mark_failed
    input: ...

edges:
  - from: charge_customer
    to: send_receipt
    condition: on_success
  - from: charge_customer
    to: notify_finance_failed
    condition: on_error
    error_matcher: any
  - from: charge_customer
    to: rollback_order
    condition: on_error
    error_matcher: { kind: Permanent }
```

### Edge condition

```rust
// nebula-workflow/src/connection.rs

pub enum EdgeCondition {
    /// Unconditional — always follow after predecessor terminal.
    Always,
    
    /// Follow only if predecessor succeeded.
    OnSuccess,
    
    /// Follow only if predecessor failed.
    /// `error_matcher` can filter which error kinds trigger this edge.
    OnError { matcher: ErrorMatcher },
    
    /// Custom expression evaluated on predecessor's output.
    When { expression: String },
}

pub enum ErrorMatcher {
    /// Any error triggers the edge.
    Any,
    
    /// Only Transient errors (though after R2 exhausts, this is unusual).
    Transient,
    
    /// Only Permanent errors.
    Permanent,
    
    /// Only cancelled executions.
    Cancelled,
    
    /// Match by nebula-error ErrorCode.
    ByCode(ErrorCode),
    
    /// Match by nebula-error ErrorCategory.
    ByCategory(ErrorCategory),
}
```

### Invariant: no cycles

Canon §10 requires workflow to be a DAG. `on_error` edges **must not** create cycles.

**Validation at workflow save:**

```rust
pub fn validate_workflow(def: &WorkflowDefinition) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    
    // Build graph
    let graph = build_dependency_graph(def);
    
    // Check acyclicity
    if let Err(cycle) = petgraph::algo::toposort(&graph, None) {
        errors.push(ValidationError::CycleDetected {
            nodes: cycle,
            hint: "retry same node via cycle is not supported; use retry_policy instead",
        });
    }
    
    // ... other checks
    
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
```

If user tries to create `A → on_error → A`, workflow validator rejects at save time with clear error. No runtime cycle detection needed.

**Why no cycles:** R2 (engine retry) handles retrying same node. DAG cycles are a different feature with different semantics (Airflow-style loops), not simulated retry.

### Flow with R3

```
node_a fails terminally (all R2 attempts exhausted or non-retryable)
  ↓
execution_nodes: node_a status='Failed'
  ↓
Engine looks for outgoing edges from node_a
  Finds: node_a --on_error{Permanent}--> rollback_order
  Error was Permanent → edge matches
  ↓
Engine dispatches rollback_order as new node
  INSERT execution_nodes (logical_node_id='rollback_order', attempt=1, ...)
  ↓
rollback_order runs, succeeds or fails per its own retry_policy
```

**on_error edges do not prevent execution failure if no edge matches.** If node_a has `on_success → send_receipt` but no `on_error` edge, execution fails when node_a fails (no alternative path).

## R4 — manual workflow restart

Two sub-variants:

### R4a — Restart from scratch (v1)

API endpoint creates a **new** execution:

```
POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/restart
Body: { "from": "scratch" }

Response:
{
    "new_execution_id": "exec_02J9...",
    "workflow_version_id": "wfv_01J9...",  // pinned to same version as source
    "source": { "RestartOf": { "parent_execution_id": "exec_01J9..." } }
}
```

- Same workflow version
- Same input
- **Fresh execution_id** — new idempotency keys everywhere
- Parent execution marked as `RestartedAs(new_id)` for audit trail
- Completed nodes run again — this may duplicate non-idempotent side effects, author's responsibility via idempotency keys

**When to use:** full do-over after investigating failure, accepting possible duplicate side effects if action authors didn't use idempotent keys.

### R4b — Restart from failed node (v2, `planned`)

```
POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/restart
Body: { "from": "failed_node", "node_id": "charge_customer" }
```

- Same execution_id, new `restart_generation` counter
- Completed nodes **skipped** — their outputs reused from storage
- Failed and downstream nodes re-run
- Complex and risky (skipped node's output may be stale)

Deferred to v2 until strong need from operators. Airflow's `clear task` has this, operators report mixed feelings.

## Cancel × Retry interaction

**Rule: cancel wins over retry.**

- Node is `Running` and gets cancelled → current attempt marked `Cancelled`, no retry scheduled
- Node is `PendingRetry` and execution gets cancelled → row marked `Cancelled`, `wake_at` cleared, retry never fires
- Retry scheduled with long delay, execution cancelled while waiting → same: row marked `Cancelled`
- Cancel fires while action is mid-retry in R1 (nebula-resilience loop) → `ctx.cancellation.cancelled()` future fires, action exits, returns `Err(Cancelled)`, R2 sees `Cancelled`, does not schedule retry

**Idempotency key during cancel + retry:** cancel terminates cleanly. If external side effect had been initiated but response not received, the attempt's idempotency key is known. If user later manually restarts (R4a), new execution has different keys, no automatic reconciliation — operator's responsibility.

## Data model

### `execution_nodes` additions for retry

```sql
-- From spec 16 foundation, relevant columns shown
CREATE TABLE execution_nodes (
    id                 BYTEA PRIMARY KEY,
    execution_id       BYTEA NOT NULL,
    logical_node_id    TEXT NOT NULL,
    attempt            INT NOT NULL,
    status             TEXT NOT NULL,   -- Running/Succeeded/Failed/Cancelled/PendingRetry/Suspended
    started_at         TIMESTAMPTZ,
    finished_at        TIMESTAMPTZ,
    input              JSONB,
    output             JSONB,
    error_kind         TEXT,            -- 'transient'/'permanent'/'cancelled'/'fatal'/'timeout'
    error_message      TEXT,
    error_retry_hint_ms BIGINT,         -- from TransientWithHint
    idempotency_key    TEXT UNIQUE,     -- exec:node:attempt
    
    -- Retry tracking
    retry_policy_hash  BYTEA,           -- hash of policy used for this attempt (for audit)
    wake_at            TIMESTAMPTZ,     -- NULL unless status=PendingRetry
    
    -- ...
    version            BIGINT NOT NULL,
    UNIQUE (execution_id, logical_node_id, attempt)
);

CREATE INDEX idx_execution_nodes_pending_retry
    ON execution_nodes (wake_at)
    WHERE status = 'PendingRetry';
```

### `executions` parent-child for R4a

```sql
ALTER TABLE executions
    ADD COLUMN restarted_from BYTEA REFERENCES executions(id);

CREATE INDEX idx_executions_restart_chain
    ON executions (restarted_from)
    WHERE restarted_from IS NOT NULL;
```

## Configuration surface

```toml
[retry]
# Default retry policy for actions that don't override
default_max_attempts = 3
default_backoff = "exponential_jitter"
default_initial_delay = "1s"
default_max_delay = "60s"
default_total_timeout = "10m"
default_retry_on = "classify_based"

# Scheduler polling for PendingRetry rows
scheduler_poll_interval = "1s"

# Safety caps
max_attempts_cap = 20            # absolute upper bound (reject policies above this)
max_total_timeout = "24h"        # even with huge retry_policy, never retry for more than this
```

## Testing criteria

**Unit tests:**
- `RetryPolicy::default()` matches spec defaults
- `BackoffStrategy::delay_for(attempt)` correctly computes all strategies
- `should_retry` decision matrix for all combinations
- `ActionError::is_retryable` is correct for all variants
- Retry hint from `TransientWithHint` overrides backoff

**Integration tests (critical):**

1. **Transient failure with retry:** action returns `Err(Transient)` on first 2 attempts, `Ok` on third → execution succeeds, three rows in `execution_nodes` (attempt 1 Failed, attempt 2 Failed, attempt 3 Succeeded)
2. **Permanent failure no retry:** action returns `Err(Permanent)` → one attempt, Failed, no retry scheduled
3. **Retry budget exhausted by total_timeout:** action retries succeed too slowly, total_timeout fires → final attempt Failed, no more retries
4. **Retry budget exhausted by max_attempts:** reach max without success → final attempt Failed
5. **Retry hint honored:** action returns `TransientWithHint { retry_after: 30s }` → next attempt wake_at is 30s, not backoff default
6. **Cancel during PendingRetry:** retry scheduled, execution cancelled before wake → retry never fires, marked Cancelled
7. **Cancel mid-attempt:** action running, cancelled, `Err(Cancelled)` → no retry, status Cancelled
8. **R3 edge on error:** workflow with `A --on_error--> B`, A fails → B runs
9. **R4a restart from scratch:** failed execution, restart → new execution, fresh IDs
10. **Idempotency key per attempt:** three attempts of same node have three distinct keys
11. **Retry scheduler picks up PendingRetry on wake_at:** row with `wake_at = past` gets claimed and run

**Property tests:**
- Backoff never exceeds `max` for exponential strategies
- Jitter is bounded: `0 <= delay <= max`
- Total elapsed retry time ≤ `total_timeout + last_delay`

**Chaos tests:**
- Random cancel interleaved with retry scheduling — no stuck retries
- Worker crash during PendingRetry — next worker picks it up at wake_at (via lease expiration, spec 17)

## Performance targets

- Retry decision (`should_retry`): **< 10 µs**
- Schedule new retry attempt: **< 5 ms p99** (single INSERT + UPDATE in transaction)
- Retry scheduler poll: **< 10 ms p99** (indexed query)
- Backoff computation: **< 1 µs** per call

## Module boundaries

| Component | Crate |
|---|---|
| `RetryPolicy`, `BackoffStrategy`, `RetryClassifier` | `nebula-action` |
| `ActionError` + variants + `is_retryable` | `nebula-action` |
| `should_retry` decision logic | `nebula-runtime` |
| Retry scheduler (polls PendingRetry) | `nebula-engine` (part of dispatcher loop §17) |
| `execution_nodes` CRUD for retry | `nebula-storage` |
| R3 edge evaluation | `nebula-engine` (workflow executor) |
| R4 restart endpoint | `nebula-api` |
| `nebula-resilience` (R1) | `nebula-resilience` (existing) |

## Canon §11.2 rewrite (for fold-in)

Replace current text with:

> **§11.2 Retry.** Nebula has a four-layer retry cascade. Each layer recovers from failures within its scope; unrecovered failures propagate to the layer above.
>
> **R1 — in-action retry via `nebula-resilience`.** Authors wrap outbound calls in retry policies inside action code. In-memory, fast, author-controlled. Good for transient network blips and rate limits within a single attempt.
>
> **R2 — engine attempt retry.** Configured per action via `ActionMetadata::retry_policy`. Classification via `Classify::retryable()` + explicit `ActionError` variants (`Transient`, `Permanent`, `Cancelled`, `Fatal`, `Timeout`). Each retry is a new row in `execution_nodes` with a new `idempotency_key` (`{execution_id}:{node_id}:{attempt}`). Persisted, survives process restart. Bounded by `max_attempts` and `total_timeout`. Remote retry hints (`Retry-After`) honored.
>
> **R3 — DAG `on_error` edges.** Workflow authors add edges that route to alternative nodes on failure. `ErrorMatcher` filters which error kinds trigger each edge. **No cycles** — canon §10 DAG invariant preserved. Retrying the same node is R2's job, not R3's.
>
> **R4a — manual restart from scratch.** Operator API endpoint creates a new execution with fresh IDs. Accepts possible duplicate side effects for non-idempotent actions.
>
> **R4b — restart from failed node.** `planned v2`.
>
> **Cancel always wins over retry.** Cancel terminates in-flight attempts and prevents scheduled retries from firing.
>
> **§11.2 is no longer a false capability** — R2 is implemented end-to-end with persisted attempt accounting.

## Open questions

- **Custom retry classifier per-error-code** — instead of binary retryable/not, allow per-code policy? Adds complexity; probably YAGNI.
- **Exponential backoff with budgeted total time vs max delay** — we have both. Is one enough? Leaving both as they solve different problems.
- **Configurable retry jitter amount** — currently jitter is «full jitter» (0..=delay). AWS-style «equal jitter» or «decorrelated jitter» could be alternatives. v2 tunable.
- **Retry scheduler: separate loop or part of dispatcher?** Integrated in spec 17 dispatcher via unified claim query — single scan handles PendingRetry too. No separate service.
- **Deadletter queue for actions that exhausted all retries** — useful for manual replay and monitoring. Could be a view over `execution_nodes WHERE status='Failed'`, or a dedicated table. Deferred — view is enough for v1.
