# Spec 08 — Cancellation cascade (hierarchical structured concurrency)

> **Status:** draft
> **Canon target:** §12.2 (extend), §12.7 (cross-ref)
> **Depends on:** 06 (IDs)
> **Depended on by:** 09 (retry — cancel interacts with retry), 14 (stateful — on_cancel hook), 17 (multi-process)

## Problem

«Cancel» is the easiest feature to claim and one of the hardest to get right. It has three orthogonal layers that users conflate:

1. **Who requests it** — user clicks UI, API call, execution timeout, process shutdown, org deletion
2. **How the signal travels** — from API to storage to engine to runtime to action code
3. **What happens in-flight** — HTTP call mid-flight, DB transaction in progress, external side effect partially applied

Canon §12.2 requires durable control plane (`execution_control_queue`). That solves the **signal transport** part. This spec solves the **in-process propagation** part, and the **contract for action authors** so they do cooperative cleanup rather than being surprised.

Physical constraint: **Tokio has no preemptive cancellation.** Dropping a future only works at `.await` points. A busy loop is uninterruptible. Every cancel design must account for this.

## Decision

**Two-phase cooperative cancel with hierarchical tokens, process-wide grace waterfall, and escalation kill.** Cancel and terminate are two different user-facing actions with different RBAC and semantics. Action authors use a safe API (`ctx.cancellation.*`), never touch raw tokens.

## Architecture diagram

```
┌────────────────────────────────────────────────────────────────┐
│ Process                                                          │
│   process_token: CancellationToken (root)                        │
│   process_tracker: TaskTracker                                   │
│   grace: 60s                                                      │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ Engine                                                     │   │
│  │   engine_token: process_token.child_token()                │   │
│  │   engine_tracker: TaskTracker                              │   │
│  │   grace: 45s                                                │   │
│  │                                                              │   │
│  │  ┌─────────────────────────────────────────────────┐    │   │
│  │  │ Execution                                         │    │   │
│  │  │   exec_token: engine_token.child_token()          │    │   │
│  │  │   exec_tracker: TaskTracker                       │    │   │
│  │  │   grace: 30s                                       │    │   │
│  │  │                                                     │    │   │
│  │  │  ┌───────────────────────────────────────┐     │    │   │
│  │  │  │ Node attempt                            │     │    │   │
│  │  │  │   node_token: exec_token.child_token()  │     │    │   │
│  │  │  │   node_tracker: TaskTracker             │     │    │   │
│  │  │  │   grace: ActionMetadata::cancel_grace   │     │    │   │
│  │  │  │          (default 30s, max 5m)          │     │    │   │
│  │  │  │                                           │     │    │   │
│  │  │  │  ┌─────────────────────────────────┐ │     │    │   │
│  │  │  │  │ Action::execute                  │ │     │    │   │
│  │  │  │  │   ctx.cancellation ──► node_token│ │     │    │   │
│  │  │  │  │                                    │ │     │    │   │
│  │  │  │  │  ctx.spawn(future) ──────────┐   │ │     │    │   │
│  │  │  │  │  ctx.scope() { ... }         │   │ │     │    │   │
│  │  │  │  │                                │   │ │     │    │   │
│  │  │  │  │  ┌─────────────────────┐     │   │ │     │    │   │
│  │  │  │  │  │ Author's sub-task    │     │   │ │     │    │   │
│  │  │  │  │  │ (child of node_token)│◄────┘   │ │     │    │   │
│  │  │  │  │  └─────────────────────┘         │ │     │    │   │
│  │  │  │  └─────────────────────────────────┘ │     │    │   │
│  │  │  └───────────────────────────────────────┘     │    │   │
│  │  └─────────────────────────────────────────────────┘    │   │
│  └─────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────┘

Propagation:
  Cancel signal:  Process → Engine → Execution → Node → Action → sub-tasks  (DOWN)
  Shutdown wait:  sub-tasks → Action → Node → Execution → Engine → Process  (UP)
  Grace periods: 60s        45s       30s         node::cancel_grace
                 (always: parent_grace >= child_grace)
```

## Layer lifecycle primitive

```rust
// nebula-core/src/lifecycle.rs
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// One layer in the cancellation hierarchy.
pub struct LayerLifecycle {
    pub token: CancellationToken,
    pub tasks: TaskTracker,
}

impl LayerLifecycle {
    /// Root layer (process level).
    pub fn root() -> Self {
        Self {
            token: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }
    }

    /// Child layer — inherits cancellation from parent.
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            tasks: TaskTracker::new(),
        }
    }

    /// Two-phase graceful shutdown.
    /// 1. Signal cancel (sets token, children see it).
    /// 2. Stop accepting new work (close tracker).
    /// 3. Wait for children up to grace period.
    /// 4. Return outcome for caller escalation decision.
    pub async fn shutdown(&self, grace: Duration) -> ShutdownOutcome {
        self.token.cancel();
        self.tasks.close();

        tokio::select! {
            _ = self.tasks.wait() => ShutdownOutcome::Graceful,
            _ = tokio::time::sleep(grace) => ShutdownOutcome::GraceExceeded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Graceful,      // all children completed within grace
    GraceExceeded, // grace period elapsed, children still running
}
```

## Grace waterfall

Each layer's grace period is **strictly shorter** than its parent's, so parent escalation never fires before child escalation had a chance.

| Layer | Default grace | Source of config |
|---|---|---|
| Process | 60s | `NEBULA_SHUTDOWN_TIMEOUT` env, default 60 |
| Engine | 45s | engine config, default 45 |
| Execution | 30s | per-execution, default 30 |
| Node | 30s | `ActionMetadata::cancel_grace`, default 30, max 5 minutes |
| Sub-tasks | bounded by node grace | not configurable |

**Rule:** `process_grace > engine_grace > exec_grace >= node_grace`. Exec grace equals node grace in default config because nodes do the actual work — there's no additional work above nodes in an execution.

**K8s integration:**

```yaml
terminationGracePeriodSeconds: 900  # 15 minutes — long enough for any drain scenario
```

K8s SIGTERM → process calls `LayerLifecycle::shutdown(60s)`. If process shutdown within 60s → success, K8s sees clean exit. If not → K8s SIGKILL after `terminationGracePeriodSeconds`. Process-level grace is the inner budget; K8s gives us an outer budget.

## Action author API

Authors never see raw `CancellationToken`. They see a constrained interface:

```rust
// nebula-action/src/context.rs

pub struct ActionContext {
    // ... existing fields
    pub cancellation: CancellationSignal,
    pub tasks: ActionTasks,  // for spawning child tasks
}

pub struct CancellationSignal {
    token: CancellationToken,  // private — not accessible to author
}

impl CancellationSignal {
    /// Non-blocking check. Use in loops.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Returns `Err(ActionError::Cancelled)` if cancel requested.
    /// Use at natural checkpoints in the action.
    pub fn check(&self) -> Result<(), ActionError> {
        if self.token.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Future that resolves when cancel is requested.
    /// Use with `tokio::select!` to race against ongoing I/O.
    pub fn cancelled(&self) -> WaitForCancellation<'_> {
        WaitForCancellation { fut: self.token.cancelled() }
    }

    // Raw token NOT exposed to author — prevents foot-guns.
    pub(crate) fn internal_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

pub struct ActionTasks {
    tracker: TaskTracker,      // private, managed by runtime
    parent_token: CancellationToken,
}

impl ActionTasks {
    /// Spawn an async task tied to this action's cancellation.
    /// Task is automatically cancelled when action's node_token fires.
    /// Task is automatically awaited before action is considered complete.
    pub fn spawn<F, T>(&self, fut: F) -> ScopedHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let token = self.parent_token.child_token();
        let handle = self.tracker.spawn(async move {
            tokio::select! {
                result = fut => TaskResult::Completed(result),
                _ = token.cancelled() => TaskResult::Cancelled,
            }
        });
        ScopedHandle { inner: handle }
    }

    /// Scoped fan-out — spawn several tasks, wait for all to complete.
    pub fn scope(&self) -> ActionScope<'_> {
        ActionScope {
            tracker: TaskTracker::new(),
            parent_token: self.parent_token.clone(),
        }
    }
}

pub struct ActionScope<'a> {
    tracker: TaskTracker,
    parent_token: CancellationToken,
    _parent: std::marker::PhantomData<&'a ()>,
}

impl<'a> ActionScope<'a> {
    pub fn spawn<F, T>(&self, fut: F)
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let token = self.parent_token.child_token();
        self.tracker.spawn(async move {
            tokio::select! {
                result = fut => Some(result),
                _ = token.cancelled() => None,
            }
        });
    }

    /// Close scope and wait for all tasks.
    /// Propagates cancellation on grace-exceed.
    pub async fn wait(self) -> Result<(), ActionError> {
        self.tracker.close();
        self.tracker.wait().await;
        if self.parent_token.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }
}
```

## Action patterns

### Pattern 1 — periodic check in a loop

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let mut processed = 0;
    for item in input.items {
        ctx.cancellation.check()?;  // bail out here if cancelled
        process_item(item).await?;
        processed += 1;
    }
    Ok(Output { processed })
}
```

### Pattern 2 — HTTP call racing against cancel

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let http_future = reqwest::Client::new().get(&input.url).send();

    tokio::select! {
        result = http_future => {
            let resp = result.map_err(ActionError::from)?;
            let body = resp.text().await.map_err(ActionError::from)?;
            Ok(Output { body })
        }
        _ = ctx.cancellation.cancelled() => {
            // reqwest drops cleanly, TCP connection closes via Drop
            Err(ActionError::Cancelled)
        }
    }
}
```

### Pattern 3 — fan-out with scope

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let scope = ctx.tasks.scope();
    
    for url in input.urls {
        scope.spawn(async move {
            // Each sub-task inherits cancellation via scope
            fetch_and_process(url).await
        });
    }
    
    scope.wait().await?;  // cancellation propagates, tasks drained
    Ok(Output { /* aggregated */ })
}
```

### Pattern 4 — DB transaction past the point of no return

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // Pre-transaction work can be cancelled
    ctx.cancellation.check()?;
    let prepared = prepare_data(&input).await?;

    let mut tx = pool.begin().await?;

    // Inside transaction — we're past the point of no return
    // We commit or rollback deterministically regardless of cancel
    // (commit is fast, cancel doesn't help here)
    let result = do_work(&mut tx, prepared).await?;
    tx.commit().await?;

    Ok(Output::from(result))
}
```

**Rule of thumb:** check `cancellation` before starting expensive or irreversible work. Once past the point of no return, finish — don't leave inconsistent state.

### Pattern 5 — stream consumption with clean shutdown

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    let mut stream = open_stream(&input).await?;
    let mut count = 0;
    
    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(item) => {
                        handle(item).await?;
                        count += 1;
                    }
                    None => break,
                }
            }
            _ = ctx.cancellation.cancelled() => {
                stream.close().await?;  // graceful stream shutdown
                return Err(ActionError::Cancelled);
            }
        }
    }
    
    stream.close().await?;
    Ok(Output { count })
}
```

## Anti-patterns (forbidden)

### ❌ Bare `tokio::spawn`

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // WRONG — spawned task not tied to cancellation or tracker
    tokio::spawn(async {
        long_running_work().await;
    });
    
    Ok(Output { /* ... */ })
    // The spawned task continues running after action returns!
    // On cancel, this task has no token to check, runs forever or until process dies.
}
```

**Fix:** use `ctx.tasks.spawn(...)` instead. The spawned task is tied to node_token and awaited before action completes.

**Enforcement:** author docs mark `tokio::spawn` as anti-pattern. Clippy lint (custom if possible) flags it in action modules. Code review catches the rest.

### ❌ Ignoring `ctx.cancellation`

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // WRONG — never checks cancellation
    loop {
        do_forever().await;
    }
}
```

**Fix:** insert `ctx.cancellation.check()?` at least once per loop iteration.

**Enforcement:** runtime wraps action execute with timeout + cancellation escalation (below). Action that ignores cancellation will be hard-killed when grace expires, but will leak partial state.

### ❌ Catching cancel and continuing

```rust
async fn execute(&self, ctx: ActionContext, input: Input) -> Result<Output, ActionError> {
    // WRONG — swallows cancel
    match ctx.cancellation.check() {
        Err(_) => { /* log and continue */ }
        Ok(()) => {}
    }
    
    do_more_work().await?;
    Ok(Output { /* ... */ })
}
```

**Fix:** propagate `ActionError::Cancelled` via `?`. Cancel should always propagate to the caller.

## Runtime responsibility

Runtime wraps `action.execute()`:

```rust
// nebula-runtime/src/executor.rs
pub async fn run_action(
    action: Arc<dyn Action>,
    ctx: ActionContext,
    input: Value,
    cancel_grace: Duration,
) -> Result<Value, ActionError> {
    let node_token = ctx.cancellation.internal_token();
    let action_future = action.execute(ctx.clone(), input);

    tokio::select! {
        result = action_future => {
            // Action returned naturally — let pool finish child tasks
            let inner_shutdown = ctx.tasks.tracker.close();
            tokio::time::timeout(
                Duration::from_secs(5),
                ctx.tasks.tracker.wait()
            ).await.ok();
            result
        }
        _ = node_token.cancelled() => {
            // Cancel requested — wait for graceful return within grace period
            match tokio::time::timeout(cancel_grace, action_future).await {
                Ok(result) => result,  // action returned (possibly with Err(Cancelled))
                Err(_) => {
                    // Grace exceeded — escalate
                    // Drop the future (via TaskTracker::close + abort)
                    // This causes RAII cleanup on dropped futures
                    ctx.tasks.tracker.close();
                    // Note: future is already borrowed by the select!,
                    // need different structure — see below
                    Err(ActionError::CancelledEscalated)
                }
            }
        }
    }
}
```

**Correction:** the above has a `select!` issue (action_future can't be polled twice). Real implementation uses `JoinHandle`:

```rust
pub async fn run_action(
    action: Arc<dyn Action>,
    ctx: ActionContext,
    input: Value,
    cancel_grace: Duration,
) -> Result<Value, ActionError> {
    let node_token = ctx.cancellation.internal_token();
    let handle = tokio::spawn({
        let ctx = ctx.clone();
        async move { action.execute(ctx, input).await }
    });

    // Wait for either:
    // - action to finish naturally
    // - cancel + grace + forced abort
    let wait_fut = async {
        // Wait for cancel
        node_token.cancelled().await;
        // Give grace period
        tokio::time::sleep(cancel_grace).await;
        // Grace exceeded — abort the task
        handle.abort();
    };

    tokio::select! {
        result = &mut Box::pin(handle) => {
            // Action completed (possibly with error or panic)
            match result {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(action_err)) => Err(action_err),
                Err(join_err) if join_err.is_cancelled() => Err(ActionError::CancelledEscalated),
                Err(join_err) => Err(ActionError::Fatal(join_err.to_string())),
            }
        }
        _ = wait_fut => {
            // This branch runs only if abort() was called
            // The select will then pick up the handle result
            unreachable!()  // abort causes handle to complete with is_cancelled=true
        }
    }
}
```

Exact mechanics depend on final runtime implementation; the contract is:

1. **If action returns Ok before cancel** → Ok is returned to caller
2. **If action returns Err(_)** (any variant) → Err is returned
3. **If cancel requested and action returns Err(Cancelled) within grace** → graceful cancel, journal «Cancelled»
4. **If cancel requested and action does not return within grace** → runtime aborts the task, journal «CancelledEscalated»
5. **If action panics** → runtime catches, journal «Fatal»

## Cancel vs Terminate — two different user actions

| Action | Endpoint | RBAC | Behavior |
|---|---|---|---|
| **Cancel** | `DELETE /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}` | `WorkspaceRunner`+ | Full graceful cascade. Goes through durable queue. 2-phase: cooperative → escalation. |
| **Terminate** | `POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{id}/terminate` | `WorkspaceAdmin`+ | Skip phase 1. Immediate abort of running tasks. Used for zombie recovery. |

**Why two:**

- Normal cancel should be graceful — user clicks «stop» in UI, expects clean shutdown with partial state recorded
- Terminate is for operators recovering from stuck state — «this execution has been Cancelling for 10 minutes and not responding, force-kill it»
- Separate RBAC prevents casual users from hard-killing state; admin action required
- Audit log records them separately for post-mortem clarity

## Cascade flow

User clicks «Cancel execution» in UI.

```
Step 1: API request
  DELETE /api/v1/orgs/acme/workspaces/prod/executions/exec_01J9...
  ↓
Step 2: Permission check
  ctx.require(Permission::ExecutionCancel)
  ↓
Step 3: Atomic state transition + control queue enqueue (canon §12.2)
  BEGIN TRANSACTION
  UPDATE executions SET status = 'Cancelling', version = version + 1
    WHERE id = ? AND version = ? AND status = 'Running'
  INSERT INTO execution_control_queue (execution_id, command, ...)
    VALUES (?, 'Cancel', ...)
  COMMIT
  ↓
Step 4: API returns 202 Accepted {"status": "cancelling"}
  User sees immediate feedback in UI.
  ↓
Step 5: Control queue worker scans
  Each node scans control queue filtered by own claimed executions.
  Interval: 2s (configurable).
  ↓
Step 6: Worker finds the cancel command
  Worker identifies this execution is local (it holds the lease).
  ↓
Step 7: Worker fires local exec_token
  exec_lifecycle.shutdown(30s).await
  - exec_token.cancel() → fires → all child node_tokens cancelled
  - exec_tracker.close() + wait
  ↓
Step 8: Each running node's action sees cancellation
  - ctx.cancellation.cancelled() future resolves
  - Action's tokio::select! branch fires
  - Action returns Err(ActionError::Cancelled)
  ↓
Step 9: Node attempt marked Cancelled in execution_nodes
  UPDATE execution_nodes SET status = 'Cancelled', finished_at = NOW(), ...
  ↓
Step 10: All nodes reached terminal → execution reaches terminal
  UPDATE executions SET status = 'Cancelled', finished_at = NOW()
  ↓
Step 11: Events emitted
  nebula-eventbus: ExecutionEvent::Cancelled { execution_id, ... }
  Subscribers (websocket, metrics, journal) receive.
  ↓
Step 12: User sees UI update via websocket
  Execution moves from "Cancelling" to "Cancelled" in live list.
```

Latency budget end-to-end:

- Step 1-4: **~50ms** (API + DB write)
- Step 5: **0-2000ms** (poll interval)
- Step 6-8: **1-100ms** (in-process token fire + next await point)
- Step 9-11: **~50ms** (DB updates + event fan-out)
- Step 12: **~100ms** (websocket delivery + UI render)

**Total: ~250ms to ~2.5s** end-to-end. Within a user-visible «instant» for typical case.

## Process shutdown flow (SIGTERM)

Process receives SIGTERM (K8s rolling deploy, `systemctl stop`, etc.):

```
Step 1: Signal handler fires
  tokio::signal::unix::signal(SignalKind::terminate()) receives
  ↓
Step 2: Process enters draining state
  process_lifecycle.shutdown(60s).await
  - process_token.cancel() → fires → cascade
  - process_tracker.close() + wait
  ↓
Step 3: All engines stop accepting new work
  engine_lifecycle observes parent cancel
  engine_tracker.close() — no new executions claimed
  ↓
Step 4: All executions cascade
  Each exec_lifecycle sees parent cancel
  - If execution can finish within grace: let it finish
  - Otherwise: cascade to nodes
  ↓
Step 5: All nodes see cancellation
  Each action's node_token fires
  Cooperative cancel runs graceful path
  ↓
Step 6: Bottom-up wait through trackers
  Actions return → node_tracker.wait() unblocks
  Nodes finish → exec_tracker.wait() unblocks
  Executions finish → engine_tracker.wait() unblocks
  Engines finish → process_tracker.wait() unblocks
  ↓
Step 7: Process exits cleanly
  std::process::exit(0)
  ↓
Step 8 (on timeout): Escalation
  If process didn't exit within 60s:
  - Engine grace already exceeded (45s)
  - Exec grace already exceeded (30s)
  - Node grace already exceeded (30s)
  - Active tasks aborted
  - Remaining dirty state lost (next process takes over via multi-worker §17)
```

**No data corruption** — durable state was already committed at last checkpoint or at last state transition. Partial work since last checkpoint is lost but will be replayed by next worker via idempotency keys.

## Edge cases

**Race: user cancels while action is about to return Ok.** Two outcomes race — whichever commits first wins. CAS on `executions.version` ensures only one transition succeeds. If Ok commits first, cancel sees `status = Succeeded`, rejects cancel gracefully. If cancel commits first, Ok commit sees `status = Cancelled`, fails CAS, discards result.

**Multiple cancel requests for same execution.** First one transitions to `Cancelling`, subsequent ones are no-ops (`status != Running`, CAS fails, API returns `200 OK` idempotently).

**Cancel during suspension (`WaitUntil`).** Suspended action has `wake_at` or `wake_signal_name` set. Cancel transitions directly to `Cancelled` — no need to wait for wake, because nothing is running.

**Cancel during retry wait.** Action is in `PendingRetry` state, waiting for `next_retry_at`. Cancel transitions to `Cancelled` — `next_retry_at` is cleared. Retry is prevented (spec 09).

**Cancel crosses worker lease expiry.** Worker A crashes mid-execution, lease expires, worker B takes over. Meanwhile cancel is requested. Both changes happen. Worker B picks up both — starts from last checkpoint, immediately sees cancel, transitions to Cancelled without continuing.

**Cancel and grace exceeded: did the side effect commit externally?** Unknown. Action was killed between «call sent» and «response received». External system may or may not have applied the effect. Canon §11.3 idempotency contract: action was supposed to use a stable idempotency key, so re-running with same key returns cached result (if external system supports it). If not, manual reconciliation needed. Documented in §15 delivery semantics.

**Process crash during cascade.** Cancel was written to durable queue. Cascade was in progress when process died. New worker takes over via lease expiration (spec 17). Sees Cancelling state + queue entry, resumes cascade. Idempotent because each layer's cancel is idempotent.

**User requests cancel for a done execution.** API returns `409 Conflict` with «execution already terminal», or `200 OK` with no-op semantics depending on UX choice. Preference: `409` for clarity.

## Data model changes

Add columns to `executions` table:

```sql
ALTER TABLE executions
    ADD COLUMN cancel_requested_at TIMESTAMPTZ,
    ADD COLUMN cancel_requested_by BYTEA,       -- user or service account
    ADD COLUMN cancel_reason TEXT,               -- optional, provided by user
    ADD COLUMN escalated BOOLEAN NOT NULL DEFAULT FALSE;
```

Same for `execution_nodes`:

```sql
ALTER TABLE execution_nodes
    ADD COLUMN escalated BOOLEAN NOT NULL DEFAULT FALSE;
```

`escalated = TRUE` means grace period expired and forced kill was used — important for post-mortem and metrics.

## Configuration surface

```toml
[runtime.cancel]
# Process-level shutdown grace (matches K8s terminationGracePeriodSeconds)
process_grace = "60s"

# Engine-level grace (must be < process_grace)
engine_grace = "45s"

# Default execution grace (per-execution override possible)
execution_grace = "30s"

# Default node grace (ActionMetadata::cancel_grace can override per-action)
default_node_grace = "30s"
max_node_grace = "5m"  # hard cap

# Control queue scan interval
control_queue_poll_interval = "2s"
```

## Testing criteria

**Unit tests:**
- `LayerLifecycle::shutdown` returns `Graceful` when children finish in time
- `LayerLifecycle::shutdown` returns `GraceExceeded` when they don't
- Child token inheritance: parent.cancel() → child.is_cancelled()
- `CancellationSignal::check()` returns `Err(Cancelled)` after token fired
- `ActionScope::wait()` propagates cancellation

**Integration tests (critical, these prove the feature works):**

1. **Simple cancel flow:** start execution with one long-running node, send cancel, assert execution reaches Cancelled within 5 seconds
2. **Cascade to nodes:** start execution with fan-out of 5 nodes, cancel, assert all 5 nodes reach Cancelled
3. **Escalation:** write a test action that ignores cancellation (busy loop), assert escalation kicks in after grace period, assert `escalated=true`
4. **Cancel during HTTP call:** action doing a 60-second mock HTTP call, cancel at 5s, assert action returns quickly with Cancelled
5. **Cancel during WaitUntil:** action suspended waiting for signal, cancel, assert immediate transition to Cancelled
6. **Process SIGTERM during running execution:** start executions, send SIGTERM, assert clean shutdown within grace
7. **Worker crash + another worker takes cancel:** crash worker mid-cancel-cascade, verify new worker completes cancel

**Property tests:**
- Child grace ≤ parent grace (checked at config load)
- Cancel is idempotent (multiple cancels of same execution same result)
- CancellationToken propagates through arbitrary child depths

**Chaos tests:**
- Random cancels interleaved with random completions — no stuck states
- Multiple cancels concurrent with retry scheduling — no race wins forever

## Performance targets

- Cancel to token fire (single layer): **< 1 ms** (in-memory channel fire)
- Cancel end-to-end (API to action sees it): **< 2.5s p99** (includes poll interval)
- Cascade depth: **< 100 µs** per layer (token fire is cheap)
- Grace waterfall overhead: **0** (parallel, not sequential waits)

## Module boundaries

| Component | Crate |
|---|---|
| `LayerLifecycle`, `ShutdownOutcome` | `nebula-core` |
| `CancellationSignal`, `ActionTasks`, `ActionScope` | `nebula-action` |
| Runtime wrapper `run_action` | `nebula-runtime` |
| `ActionError::Cancelled`, `ActionError::CancelledEscalated` | `nebula-action` |
| Control queue scanner (integration with §17) | `nebula-engine` |
| Cancel API endpoint handlers | `nebula-api` |
| Cascade coordination (exec → nodes) | `nebula-engine` |

## Migration path

**Greenfield** — no prior cancel infrastructure to migrate.

**Action authors:** the first batch of actions built against this contract must be reviewed for:
- Any bare `tokio::spawn` calls (replace with `ctx.tasks.spawn`)
- Missing `ctx.cancellation.check()` in long-running loops
- `tokio::select!` handling of `cancelled()` future

Documentation in `nebula-action` README shows the patterns above as canonical templates.

## Open questions

- **`on_cancel` hook for cleanup** — if action needs explicit cleanup action (e.g., drop a database lock, unset a flag), should there be a hook called after `Err(Cancelled)` is returned? Or is RAII `Drop` on held resources enough? Leaning: RAII is enough for v1, add `on_cancel` hook only if real need appears.
- **Compensation logic (saga pattern)** — action that needs to undo previously-committed side effects on cancel. Temporal supports via workflow code. We'd support via DAG `on_error` edges (spec 09 R3) and/or explicit compensation workflows. Not in v1 scope.
- **Cancel vs Terminate RBAC boundary** — is WorkspaceAdmin strict enough for terminate, or does it require OrgAdmin? Leaning: WorkspaceAdmin is enough, audit log records it.
- **Partial result preservation** — when cancel fires mid-execution of a stateful action, the author may want to commit partial progress before returning. Spec 14 (stateful actions) addresses this via `on_cancel` hook on `StatefulAction`.
