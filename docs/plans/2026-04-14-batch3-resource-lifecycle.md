# Batch 3 — Resource / Credential / Config Lifecycle Hardening

**Date:** 2026-04-14
**Branch (default):** `fix/resource-lifecycle-batch3`
**Author:** architect (for rust-senior to implement)
**Issues:** #302, #314, #315, #318, #322, #323

This batch fixes six HIGH-priority lifecycle / concurrency bugs across
`nebula-resource`, `nebula-credential`, and `nebula-config`. All six are
use-after-logical-drop, deadlock, leak, restart-unsafe, herd, or
cancellation-blocking bugs — i.e. none are purely cosmetic, and all of them
require real API changes (no adapters, no shims).

---

## Shared concerns

### Cross-issue coupling

1. **#318 and #323 both touch `DaemonRuntime`.**
   - #318: token reused across `start` / `stop`; finished handle never cleared.
   - #323: plain `sleep(restart_backoff)` during backoff does not observe cancel.
   - **Design them together**: a single `daemon_loop` refactor where the per-run
     cancellation token is *created inside* `start()` (not reused from the
     runtime field), and the backoff sleep races against that same token.
     Doing them sequentially would either re-introduce one of the bugs, or
     produce two competing notions of "the cancel token".

2. **#302 and #322 both touch `Manager`.**
   - #302: `graceful_shutdown` swallows drain-timeout → destroys live resources.
   - #322: `check_recovery_gate` uses a state snapshot and lets all callers
     through after `retry_at`, defeating single-probe serialization.
   - **Design them together**: both change the error-propagation contract of
     `Manager` entry points. #302 changes `graceful_shutdown(&self, _) -> ()`
     to return `Result<ShutdownReport, ShutdownError>`; #322 changes how
     `check_recovery_gate` hands back a typed gate outcome *plus*, in the
     probe path, a `RecoveryTicket` that the acquire must resolve/fail.
     They share one consistent error enum style (`thiserror` with
     `#[error]` per variant, `Classify`-able where useful).

3. **#314 and #315 are independent** — one is a credential semaphore
     validation, the other is a config-watcher drop semantics fix. They have
     no type-level overlap with each other or with the resource work.

### API breakage scope

| Crate              | Public breakage | Downstream rebuild |
|--------------------|-----------------|--------------------|
| `nebula-resource`  | `Manager::graceful_shutdown` signature, `ShutdownConfig` fields, `check_recovery_gate` signature (private), acquire error path | `engine`, `api`, `action`, `plugin`, `sdk` (recompile only — no call-site changes except `graceful_shutdown` callers) |
| `nebula-credential`| `RefreshCoordinator::with_max_concurrent` return type (`Self` → `Result<Self, RefreshConfigError>`) | `credential` internal + any explicit caller in `examples/` |
| `nebula-config`    | `ConfigWatcher` trait adds one sync method (`cancel_on_drop`) or accepts `CancellationToken` on `start_watching`; `Config::drop` still sync | `api`, `engine`, `runtime` (recompile only) |

### PR split decision

**Ship as ONE PR: `fix/resource-lifecycle-batch3`.**

Rationale:
- The four resource bugs (#302, #318, #322, #323) cannot cleanly split: they
  overlap on `Manager` and `DaemonRuntime` in ways that would force one PR to
  depend on the other.
- #314 (credential) and #315 (config) are small, localized, and reviewed
  faster alongside rather than as two micro-PRs. Combined churn is ~400 LOC.
- A single PR simplifies the regression-test story (one suite runs once).
- All changes fall inside **non-persistent in-memory APIs** — no schema
  migration, no forward-compat concern, breaking-change label is sufficient.

If review pressure requires a split, the *only* clean seam is:
- **PR A:** `fix/resource-lifecycle-batch3` — issues #302, #318, #322, #323
- **PR B:** `fix/credential-config-lifecycle-batch3b` — issues #314, #315

Do not split finer than that.

---

## Issue #302 — `graceful_shutdown` proceeds after drain timeout

**File:** `crates/resource/src/manager.rs`

### Root cause
`wait_for_drain` returns `()`; its timeout branch only logs `tracing::warn!`,
so `graceful_shutdown` cannot tell "drained" from "gave up". It unconditionally
calls `registry.clear()`, dropping live `ManagedResource`s while `ResourceHandle`s
are still outstanding.

### Fix strategy

1. Give `wait_for_drain` a typed result and propagate it.
2. Make drain-timeout policy explicit in `ShutdownConfig`.
3. Return a typed `Result<ShutdownReport, ShutdownError>` from `graceful_shutdown`.
4. Add an idempotency guard.
5. Bound the `release_queue_handle.lock()` wait.

```rust
// crates/resource/src/manager.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrainTimeoutPolicy {
    /// On drain timeout, return `ShutdownError::DrainTimeout` *without*
    /// clearing the registry. Live handles remain valid; caller decides.
    /// This is the default — it preserves the "graceful" guarantee.
    Abort,
    /// On drain timeout, log, clear the registry anyway, and report the
    /// outstanding-handle count in `ShutdownReport`. Opt-in, "I know what
    /// I'm doing" escape hatch for supervisors that must exit.
    Force,
}

impl Default for DrainTimeoutPolicy {
    fn default() -> Self { Self::Abort }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownConfig {
    pub drain_timeout: Duration,
    pub on_drain_timeout: DrainTimeoutPolicy,
    /// Cap on how long Phase 4 will wait for release-queue workers.
    pub release_queue_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            on_drain_timeout: DrainTimeoutPolicy::Abort,
            release_queue_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// Handles still outstanding when the drain phase finished.
    /// Zero on the happy path.
    pub outstanding_handles_after_drain: u64,
    /// Whether Phase 3 (`registry.clear`) actually ran.
    pub registry_cleared: bool,
    /// Whether Phase 4 completed within `release_queue_timeout`.
    pub release_queue_drained: bool,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ShutdownError {
    #[error("graceful shutdown already in progress")]
    AlreadyShuttingDown,

    #[error(
        "drain timeout expired with {outstanding} handle(s) still active; \
         registry was NOT cleared (policy=Abort)"
    )]
    DrainTimeout { outstanding: u64 },

    #[error("release queue workers did not finish within {timeout:?}")]
    ReleaseQueueTimeout { timeout: Duration },

    #[error("internal lock acquisition timed out during shutdown")]
    LockTimeout,
}

impl Manager {
    pub async fn graceful_shutdown(
        &self,
        config: ShutdownConfig,
    ) -> Result<ShutdownReport, ShutdownError> { /* ... */ }

    // private — now returns the outstanding count on timeout
    async fn wait_for_drain(
        &self,
        timeout: Duration,
    ) -> Result<(), DrainTimeoutError>;
}

struct DrainTimeoutError { outstanding: u64 }
```

Add to `Manager`:

```rust
shutting_down: AtomicBool, // CAS-guarded at start of graceful_shutdown
```

Flow of the new `graceful_shutdown`:

1. CAS `shutting_down` false→true; on failure return `AlreadyShuttingDown`.
2. Phase 1: `self.cancel.cancel()` (always runs).
3. Phase 2: `wait_for_drain(config.drain_timeout).await`.
   - `Ok(())` → proceed.
   - `Err(DrainTimeoutError { outstanding })` →
     - If `Abort`: `return Err(ShutdownError::DrainTimeout { outstanding })`.
     - If `Force`: record into `ShutdownReport`, proceed.
4. Phase 3: `self.registry.clear()`. Record `registry_cleared = true`.
5. Phase 4: take the release-queue handle under a `tokio::time::timeout` of
   `config.release_queue_timeout`; if the *lock* times out, return
   `LockTimeout`. If the shutdown future times out, return `ReleaseQueueTimeout`.
6. Return `Ok(ShutdownReport { ... })`.

**No silent swallowing.** Every failure path is a real `Err`.

### Call-site impact

Grep for `graceful_shutdown(`:
- `examples/` (any runnable demos)
- `apps/cli`
- `crates/engine/**` (if it drives manager shutdown)
- `crates/api/**` (admin/HTTP shutdown handler)
- `crates/resource/src/manager.rs` tests

Each caller must:
- Await a `Result<ShutdownReport, ShutdownError>` and handle the variants.
- Pick `Abort` (default) or `Force` depending on whether they're under an
  orchestrator deadline. Binaries (`apps/cli`) may opt into `Force` with a
  warning-log on `DrainTimeout`; libraries never should.

### Test strategy

Tests to add in `crates/resource/src/manager.rs` (`mod tests`):

- `graceful_shutdown_abort_on_drain_timeout_preserves_registry` — acquire a
  handle, shutdown with `drain_timeout = 50ms` + `Abort`; assert `Err(DrainTimeout)`,
  assert the handle is still usable, assert `registry.contains(key)`.
- `graceful_shutdown_force_clears_registry_on_timeout` — same setup with
  `Force`; assert `Ok(ShutdownReport { registry_cleared: true, outstanding_handles_after_drain > 0, .. })`.
- `graceful_shutdown_happy_path_returns_zero_outstanding`.
- `graceful_shutdown_second_call_errors_already_shutting_down` — concurrent
  calls (`tokio::join!`); exactly one returns `Ok`, the other
  `Err(AlreadyShuttingDown)`.
- `graceful_shutdown_release_queue_timeout_returns_error` — stall workers
  manually.

---

## Issue #322 — RecoveryGate probe window is not serialized

**Files:** `crates/resource/src/manager.rs`, `crates/resource/src/recovery/gate.rs`

### Root cause
`check_recovery_gate()` inspects `gate.state()` read-only; on expired `Failed`
it returns `Ok(())` so the caller proceeds. All concurrent callers after
`retry_at` see the same snapshot and stampede the backend.

### Fix strategy

Replace the snapshot check with a CAS-based claim. The gate check must return
either:
- "no gate attached / gate idle — proceed without a ticket", or
- "you are the probe — here is a `RecoveryTicket`, resolve/fail it based on
  the acquire result", or
- a typed error.

```rust
// crates/resource/src/manager.rs (private helper)

/// Outcome of the pre-acquire gate check.
enum GateAdmission {
    /// No gate attached or gate was idle. Proceed normally; no ticket.
    Open,
    /// This caller has been granted the single probe slot. The acquire
    /// *must* call `ticket.resolve()` on Ok, or
    /// `ticket.fail_transient(_)` / `ticket.fail_permanent(_)` on Err.
    Probe(RecoveryTicket),
}

fn admit_through_gate(gate: &Option<Arc<RecoveryGate>>) -> Result<GateAdmission, Error> {
    let Some(gate) = gate else { return Ok(GateAdmission::Open) };

    match gate.try_begin() {
        Ok(ticket) => Ok(GateAdmission::Probe(ticket)),
        Err(TryBeginError::AlreadyInProgress(_waiter)) => Err(Error::transient(
            "backend recovery in progress, retry later",
        )),
        Err(TryBeginError::RetryLater { retry_at }) => {
            let wait = retry_at.saturating_duration_since(Instant::now());
            Err(Error::exhausted("backend recovering", Some(wait)))
        }
        Err(TryBeginError::PermanentlyFailed { message }) => Err(Error::permanent(message)),
    }
}
```

Every `acquire_*` method in `manager.rs` changes its gate interaction to:

```rust
let admission = admit_through_gate(&managed.recovery_gate)?;
// ... run execute_with_resilience ...
match (&result, admission) {
    (Ok(_), GateAdmission::Probe(t)) => t.resolve(),
    (Err(e), GateAdmission::Probe(t)) if e.is_retryable() => {
        t.fail_transient(e.to_string());
    }
    (Err(e), GateAdmission::Probe(t)) => t.fail_permanent(e.to_string()),
    (_, GateAdmission::Open) => {} // no gate, or no-gate path
}
```

Delete `trigger_recovery_on_failure` — it is now dead (the ticket lives through
the acquire, so failure handling is structural, not post-hoc).

`GateState::Idle` path of `try_begin` already grants a ticket, so the
"healthy backend, no recovery needed" case still just flows through as
`Probe(ticket)` and `resolve()`s on Ok. This is fine: `try_begin` on Idle is
cheap (one CAS), and it means every acquire under a gate always holds
ticket-ownership end-to-end. No probe herd is possible at the boundary.

### Breaking changes
None externally visible. `check_recovery_gate` and `trigger_recovery_on_failure`
are both private. `RecoveryGate::try_begin`, `RecoveryTicket`, and
`TryBeginError` are already public and unchanged.

### Test strategy
Add in `crates/resource/src/manager.rs` tests (or a new
`recovery_gate_stampede.rs` integration test):

- `probe_boundary_serializes_callers_under_herd` — register a resource whose
  acquire sleeps 50ms and returns a transient error; attach a gate with
  `base_backoff = 25ms`; `tokio::spawn` 64 concurrent acquires; after the
  first fails the gate enters `Failed`; wait for `retry_at`; spawn another
  64 concurrent acquires; assert that **exactly one** acquire actually
  invoked the runtime's acquire path (use an `AtomicUsize` counter in a
  fake `Resource` impl), the other 63 got `Err(AlreadyInProgress|RetryLater)`.
- `idle_gate_grants_ticket_and_resolves_on_ok`.
- `failing_acquire_marks_gate_fail_transient_via_ticket`.

---

## Issue #318 — `DaemonRuntime` not restart-safe

**Files:** `crates/resource/src/runtime/daemon.rs`, `crates/resource/src/topology/daemon.rs`

### Root cause
`DaemonRuntime` stores a single `CancellationToken` in its field. `stop()`
cancels it; the next `start()` clones it *already-cancelled*, so
`daemon_loop` exits immediately. On natural exit, the finished `JoinHandle`
is never cleared from `self.handle`, so the next `start()` hits the
`"daemon is already running"` guard.

### Fix strategy

Make the cancellation token and the `JoinHandle` **per-run**. The runtime
field stores only the per-run state under a single lock.

```rust
// crates/resource/src/runtime/daemon.rs

pub struct DaemonRuntime<R: Resource> {
    config: Config,
    /// Parent token for the whole runtime (never cancelled by `stop`).
    /// Cancelled only by the parent Manager (its shutdown token).
    parent_cancel: CancellationToken,
    inner: Mutex<Option<DaemonRun>>,
    _phantom: PhantomData<R>,
}

struct DaemonRun {
    /// Per-run token, cancelled by `stop()`.
    cancel: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}
```

- `new` renames `cancel` → `parent_cancel`.
- `cancel_token()` returns `&self.parent_cancel` (still meaningful as a read-only
  child source for external observers; **do not** cancel it from `stop()`).
- `start()`:
  1. Acquire the inner mutex.
  2. If `Some(run)` is present and `!run.handle.is_finished()`, return
     `Err(Error::permanent("daemon is already running"))`.
  3. If `Some(run)` is present but `run.handle.is_finished()`, drop it
     (stale-handle cleanup — this satisfies #318's "finished handle
     never cleared").
  4. Build a fresh per-run `cancel = parent_cancel.child_token()` so an
     external shutdown of `parent_cancel` still propagates.
  5. Spawn `daemon_loop(..., cancel.clone())`, store `DaemonRun { cancel, handle }`.
- `stop()`:
  1. Take `inner`'s `DaemonRun` under the lock.
  2. `run.cancel.cancel()` — **per-run only**, does not touch `parent_cancel`.
  3. `run.handle.await.ok()` — but since the user rule forbids `let _ =`,
     handle the join result with `if let Err(e) = run.handle.await { tracing::warn!(%e, "daemon join error on stop"); }`.
- A new method `pub fn is_running(&self) -> bool` for tests / diagnostics
  (locks and checks `is_finished`).

### Call-site impact
- `crates/resource/src/topology/daemon.rs` — `Config` struct unchanged.
- `crates/resource/src/runtime/` — any call-site using `DaemonRuntime::cancel_token`
  remains the same semantically (read-only parent token).
- No public API surface changes beyond `is_running`.

### Test strategy
Add in `crates/resource/src/runtime/daemon.rs` tests:

- `start_stop_start_runs_daemon_twice` — 2 successful `start`/`stop` cycles
  with a `Daemon` impl that increments a shared counter on each `run()`.
  Assert counter ≥ 2.
- `start_after_natural_exit_succeeds` — `RestartPolicy::Never`, `run` returns
  immediately; wait for handle to finish; `is_running()` false; second
  `start` succeeds.
- `concurrent_start_rejects_second` — two `start()` calls in flight on a
  running daemon; exactly one returns `Err`.
- `parent_cancel_propagates_to_running_daemon` — cancel
  `parent_cancel` externally; daemon run exits; `is_running()` false.

---

## Issue #323 — stop is delayed by `restart_backoff`

**File:** `crates/resource/src/runtime/daemon.rs`

### Root cause
`daemon_loop` uses `tokio::time::sleep(config.restart_backoff).await`
without selecting on the cancel token. `stop()` blocks for up to
`restart_backoff` before the loop notices the cancel.

### Fix strategy
Design this together with #318 so the per-run `cancel` token from #318 is
the same one the backoff race uses.

```rust
// crates/resource/src/runtime/daemon.rs (inside daemon_loop)

// Replace: tokio::time::sleep(config.restart_backoff).await;
tokio::select! {
    biased;
    _ = cancel.cancelled() => { break; }
    _ = tokio::time::sleep(config.restart_backoff) => {}
}
```

Use `biased;` so cancellation wins deterministically — critical for tight
shutdown budgets. Do this inside the same refactor commit that lands #318
so the loop signature and cancel token plumbing change exactly once.

### Breaking changes
None.

### Test strategy
- `stop_during_backoff_returns_promptly` — `Daemon::run` returns `Err`
  immediately, `restart_backoff = 10s`, `max_restarts = 10`.
  After `start`, wait ~50ms (into the backoff sleep), call `stop()`, assert
  it returns in < 500ms.

---

## Issue #314 — `RefreshCoordinator::with_max_concurrent(0)` deadlocks

**File:** `crates/credential/src/refresh.rs`

### Root cause
`Semaphore::new(0)` accepts 0 permits; any `acquire_owned().await` on it
waits forever. `with_max_concurrent` has no validation.

### Fix strategy
Return a typed error instead of clamping. Clamping hides misconfiguration;
a real error surfaces it at construction time.

```rust
// crates/credential/src/refresh.rs

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshConfigError {
    #[error("RefreshCoordinator::max_concurrent must be >= 1, got 0")]
    ZeroConcurrency,
}

impl RefreshCoordinator {
    pub fn with_max_concurrent(max: usize) -> Result<Self, RefreshConfigError> {
        if max == 0 {
            return Err(RefreshConfigError::ZeroConcurrency);
        }
        Ok(Self {
            in_flight: parking_lot::Mutex::new(HashMap::new()),
            circuit_breakers: parking_lot::Mutex::new(HashMap::new()),
            refresh_semaphore: Arc::new(tokio::sync::Semaphore::new(max)),
        })
    }

    pub fn new() -> Self {
        // DEFAULT_MAX_CONCURRENT_REFRESHES is a const = 32, statically > 0.
        // Keep `new()` infallible by calling a private infallible constructor
        // with the const value; do NOT `.unwrap()` the public fallible one.
        Self::with_max_concurrent_unchecked(DEFAULT_MAX_CONCURRENT_REFRESHES)
    }

    fn with_max_concurrent_unchecked(max: usize) -> Self {
        debug_assert!(max > 0);
        Self {
            in_flight: parking_lot::Mutex::new(HashMap::new()),
            circuit_breakers: parking_lot::Mutex::new(HashMap::new()),
            refresh_semaphore: Arc::new(tokio::sync::Semaphore::new(max)),
        }
    }
}
```

No `.unwrap()` in new code, no silent clamp. `new()` stays infallible because
it uses a const; the public fallible variant is the one that takes user input.

### Call-site impact
Grep for `with_max_concurrent(`:
- `crates/credential/src/resolver.rs` — if it constructs a coordinator with
  an input value, must handle `Result`.
- `crates/credential/src/**/tests` — test constructors.
- `examples/`
- `apps/cli`

Each must propagate the error via `?` (bubbling up to the credential store
builder, which already returns a `Result`).

### Test strategy
Add in `crates/credential/src/refresh.rs` (`mod tests`):

- `zero_max_concurrent_returns_config_error` — assert
  `with_max_concurrent(0)` returns `Err(RefreshConfigError::ZeroConcurrency)`.
- `one_max_concurrent_is_valid` — `with_max_concurrent(1)` returns `Ok`.
- `default_new_has_nonzero_permits` (already exists as
  `default_coordinator_has_default_permits` — verify it still passes
  post-refactor).

---

## Issue #315 — `PollingWatcher` leaks on owner drop

**Files:** `crates/config/src/watchers/polling.rs`, `crates/config/src/core/config.rs`, `crates/config/src/core/builder.rs`

### Root cause
`PollingWatcher`'s loop condition is `while watching.load(Relaxed)`. The only
writer of `false` is `stop_watching().await`, which `Config::drop` cannot
call (drop is sync; there is no runtime to `block_on`). The task leaks.

### Fix strategy

Bind the watcher's loop to the `Config`'s existing `cancel_token`
(`CancellationToken`). Cancelling the token is sync, so `Config::drop` can
fire it and the spawned task will exit on its next tick.

```rust
// crates/config/src/core/traits.rs

#[async_trait]
pub trait ConfigWatcher: Send + Sync {
    /// Start watching. The watcher MUST spawn tasks that observe `cancel`
    /// and exit when it fires. Required so `Config::drop` (sync) can tear
    /// down watcher tasks without blocking.
    async fn start_watching(
        &self,
        sources: &[ConfigSource],
        cancel: CancellationToken,
    ) -> ConfigResult<()>;

    async fn stop_watching(&self) -> ConfigResult<()>;

    fn is_watching(&self) -> bool;
}
```

```rust
// crates/config/src/watchers/polling.rs

impl ConfigWatcher for PollingWatcher {
    async fn start_watching(
        &self,
        sources: &[ConfigSource],
        cancel: CancellationToken,
    ) -> ConfigResult<()> {
        // ... existing CAS on `watching` ...
        // Spawn the loop with the cancel token:
        let handle = tokio::spawn(async move {
            watcher
                .start_polling_loop(sources, callback, metadata_cache, interval, cancel)
                .await;
        });
        // ...
    }
}

async fn start_polling_loop(
    &self,
    sources: Vec<ConfigSource>,
    callback: Arc<dyn Fn(ConfigWatchEvent) + Send + Sync>,
    metadata_cache: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    interval: Duration,
    cancel: CancellationToken,
) {
    // ... initial scan unchanged ...
    let mut interval_timer = tokio::time::interval(interval);
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = interval_timer.tick() => {}
        }
        // ... per-tick source scan unchanged ...
    }
    // Clear `watching` on exit so is_watching() becomes correct without
    // requiring stop_watching to be called.
    self.watching.store(false, Ordering::Release);
}
```

Remove the `watching` flag from the loop condition; keep it only as a
`is_watching()` status mirror, written on start (`true`) and on loop exit
(`false`).

```rust
// crates/config/src/core/config.rs (Drop)
impl Drop for Config {
    fn drop(&mut self) {
        // Cancelling here terminates polling tasks: they observe
        // the same token that start_watching was handed.
        self.cancel_token.cancel();
    }
}
```

```rust
// crates/config/src/core/config.rs (start_watching)
pub async fn start_watching(&self) -> ConfigResult<()> {
    if let Some(watcher) = &self.watcher {
        watcher
            .start_watching(&self.sources, self.cancel_token.child_token())
            .await?;
    }
    Ok(())
}
```

Using a child token means an explicit `stop_watching()` can still cancel the
child without affecting the parent — parent cancellation still cascades.

### Breaking changes
- `ConfigWatcher::start_watching` gains a required `cancel: CancellationToken`
  parameter. Any external implementor of this trait must update their impl.
  Today the only impls in-tree are `PollingWatcher` and `FileWatcher`
  (rust-senior: verify via `grep -n "impl ConfigWatcher"`). Both must be updated.
- No call-site churn for `Config::start_watching` public API — it constructs
  the child token internally.

### Test strategy
Add in `crates/config/src/watchers/polling.rs` (`mod tests`) or an
integration test under `crates/config/tests/`:

- `polling_task_exits_when_config_dropped` — build a `Config` with a
  `PollingWatcher`, start watching, drop the `Config`, `tokio::time::sleep`
  for `interval * 3`, assert the spawned task is gone (capture the
  `JoinHandle` via an injected sender or via `Arc::strong_count` on the
  callback captured by the task).
- `polling_task_exits_on_explicit_stop` — regression for existing behavior.
- `polling_task_exits_on_parent_cancel` — cancel `cancel_token` directly
  (not via drop), task exits.

---

## Implementation order (single PR)

rust-senior should land the work in this commit order inside
`fix/resource-lifecycle-batch3`:

1. **#314** (`credential/src/refresh.rs`) — smallest, zero-blast, warms up
   the error-type style for the rest. Makes `with_max_concurrent` fallible.
2. **#315** (`config/src/core/traits.rs` + `watchers/polling.rs` + `core/config.rs`) —
   independent of resource work. Ship here to unblock any other config PRs.
3. **#318 + #323 together** (`resource/src/runtime/daemon.rs`) — per-run
   cancel token refactor is a single commit; the backoff `select!` is added
   in the same commit to avoid re-plumbing the token twice.
4. **#302 + #322 together** (`resource/src/manager.rs`,
   `resource/src/recovery/gate.rs` usage) — new `ShutdownConfig`,
   `ShutdownError`, `ShutdownReport`, `GateAdmission` enum. Delete
   `trigger_recovery_on_failure`. Update all acquire methods.
5. Update `.project/context/crates/{resource,credential,config}.md` with the
   new invariants (per-run token, gate-ticket end-to-end ownership, typed
   shutdown result) so the Stop hook accepts the change.

---

## Non-negotiable constraints recap

- No `.unwrap()` in new code (use typed errors, `expect` only on
  statically-known invariants with an explanatory message).
- No `let _ =` discarding of `Result`. `JoinHandle::await` results become
  `if let Err(e) = ... { tracing::warn!(%e, ...) }`.
- No silent swallowing: every error path either returns `Err` or is logged
  with a reason **and** reflected in a returned status struct.
- No adapters / shims / compat layers. `graceful_shutdown`'s signature
  changes in place; callers update.
- All new types `#[non_exhaustive]` where they may grow.
