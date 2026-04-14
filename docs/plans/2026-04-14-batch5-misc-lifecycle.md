# Batch 5 — Misc Lifecycle Fixes

Date: 2026-04-14
Author: architect (Claude Opus 4.6)
Status: design — not implemented

Covers 10 open issues across config, runtime, storage, execution, engine, and
resource. These are leftover lifecycle / observability / persistence bugs that
did not belong to an earlier themed batch. The design groups them by crate
boundary and tight coupling, not by arbitrary "batch size".

Issues covered:
#294 #310 #313 — config watchers + hot reload wiring
#304 #308 — runtime stateful cancel + checkpoint
#305 — runtime duration histogram skew
#317 — storage in-memory lease TTL
#303 — execution IdempotencyManager dead path
#306 — engine NodeTask credential refresh failure mode
#309 — resource release queue saturation

---

## Shared concerns

### Coupled design: FileWatcher (#294 + #310 + #313)

All three touch `crates/config/src/watchers/file.rs` and the end-to-end
reload pipeline. They **must** be designed and shipped together, because:

- #294 is a strict subset of #310 (same race, #310 adds the blocking-callback
  hazard).
- #313 depends on what signal `FileWatcher` actually emits, and where. Fixing
  the callback plumbing first makes the reload wiring obvious.

### Coupled design: execute_stateful (#304 + #308)

Both touch `ActionRuntime::execute_stateful`. The cancellation fix (#304) and
the checkpoint fix (#308) overlap structurally: once the runtime takes a
`Option<PersistedState>` and writes back after each iteration, the iteration
boundary becomes the natural place for *both* the cancel-aware `select!` and
the checkpoint call. Designing them separately would ship two incompatible
shapes of the same loop.

### Cross-batch dependency: PR #363 (branch, not yet on main)

PR #363 (`e765f561`) reshapes `ConfigWatcher::start_watching` to:

```rust
async fn start_watching(
    &self,
    sources: &[ConfigSource],
    cancel: CancellationToken,
) -> ConfigResult<()>;
```

It wires `Config::drop` to cancel the token, and both `PollingWatcher` and
`FileWatcher` `select!` their background tasks on `cancel.cancelled()`.

**All config fixes in Batch 5 layer on top of that shape.** Batch 5 PR-A
merges *after* #363. If #363 lands first, the trait signature is already in
place and we only extend behavior inside `start_watching`. If ordering
flips, rebase PR-A on top; do not re-create the cancel token plumbing.

### PR split

| PR | Issues | Scope | Depends on |
|----|--------|-------|-----------|
| **5A** | #294 · #310 · #313 | `crates/config/src/watchers/file.rs` + `core/builder.rs` + `core/config.rs` | PR #363 |
| **5B** | #304 · #308 · #305 | `crates/runtime/src/runtime.rs`; touches `crates/action/src/stateful.rs` trait docs only for #308 | — |
| **5C** | #317 · #303 | `crates/storage/src/execution_repo.rs`; delete `crates/execution/src/idempotency.rs` (except `IdempotencyKey`) | — |
| **5D** | #306 | `crates/engine/src/engine.rs` (NodeTask) + `crates/execution/src/error.rs` (new error variant) | — |
| **5E** | #309 | `crates/resource/src/release_queue.rs` | — |

**Justification for splitting 5 ways instead of bundling:**

- 5A/5B/5C/5D/5E each touch a *single crate* as primary. Bundling crosses
  crate boundaries unnecessarily and makes reviewers page in multiple mental
  models per PR.
- 5A and 5B are each internally coupled (see above) — cannot be split further
  without shipping half-fixes.
- 5C bundles #317 and #303 because both sit in storage/execution persistence
  and both are small deletions/TTL checks — one reviewer sweep.
- 5D stays alone: it introduces a new error surface (`CredentialRefreshFailed`)
  that has to be reviewed deliberately.
- 5E stays alone: `ReleaseQueue` is a hot path with its own invariants and
  deserves a focused review.

No bundling. Each PR ships independently and can be reverted independently.

---

## PR 5A — config watchers + hot reload

### #294 + #310 — FileWatcher start race + blocking callback

**Root cause:** `FileWatcher::start_watching` uses non-atomic
`load` / `store` on `AtomicBool`; the notify callback uses `blocking_send`
into a bounded `mpsc::channel(100)`, which blocks the OS notifier thread
under burst load.

**Fix strategy:**

1. **CAS claim at entry.** Replace load/store with
   `compare_exchange(false, true, AcqRel, Acquire)`. Mirror the `PollingWatcher`
   pattern. On CAS failure return
   `Err(ConfigError::watch_error("Already watching"))`. Unwind the CAS to
   `false` on any subsequent setup failure (notify creation, watch call,
   path resolution) — the "claim but then fail" path must leave the watcher
   in `watching == false` so a retry after fixing the underlying error can
   succeed.

2. **Non-blocking callback.** Replace `blocking_send` with `try_send` plus a
   structured overflow metric. When `try_send` returns `Full`, increment a
   `FileWatcher::dropped_events: AtomicU64` counter and log at power-of-two
   intervals (same pattern as `ReleaseQueue`). The notify callback thread
   must never block — losing a filesystem event is preferable to stalling
   the kernel notifier.

3. **Channel capacity bump.** Raise `mpsc::channel(100)` to `mpsc::channel(512)`
   to absorb normal deploy-storm bursts. Not a fix on its own; pairs with (2)
   for observability when bursts exceed 512.

**API/code shape:**

```rust
// crates/config/src/watchers/file.rs
pub struct FileWatcher {
    // ...existing fields...
    /// Count of notify events dropped because the forwarding channel was
    /// full. Exposed via `dropped_events()` for tests and dashboards.
    dropped_events: Arc<AtomicU64>,
}

impl FileWatcher {
    /// Observability hook — number of filesystem events dropped due to
    /// forwarding channel saturation since the watcher was created.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ConfigWatcher for FileWatcher {
    async fn start_watching(
        &self,
        sources: &[ConfigSource],
        cancel: CancellationToken,          // from #363
    ) -> ConfigResult<()> {
        // Atomic claim — exactly one caller proceeds.
        if self
            .watching
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ConfigError::watch_error("Already watching"));
        }

        // Setup guard that unwinds `watching -> false` on any early-return
        // before the notifier is fully wired.
        let claim_guard = scopeguard::guard(&self.watching, |w| {
            w.store(false, Ordering::Release);
        });

        // ...existing channel / path-mapping / notify setup...
        // Notify closure uses try_send with overflow counter:
        let dropped = Arc::clone(&self.dropped_events);
        let tx_clone = tx.clone();
        let notify_cb = move |res: Result<Event, notify::Error>| {
            // ...build ConfigWatchEvent...
            match tx_clone.try_send(event) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let n = dropped.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_power_of_two() {
                        nebula_log::warn!(
                            dropped_total = n,
                            "FileWatcher forwarding channel full; dropping fs event"
                        );
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // Forward task exited — benign during shutdown.
                }
            }
        };

        // ...finish wiring, start debounced forwarder with `cancel` select...

        // All setup succeeded — defuse the unwind guard.
        std::mem::forget(claim_guard);
        Ok(())
    }
}
```

**Call-site impact:** none. `ConfigWatcher::start_watching` signature
already changed in #363. `start_watching` still returns
`Err("Already watching")` on duplicate — the error message is load-bearing
for an existing test.

**Test strategy:**

```rust
// crates/config/tests/watcher_file_test.rs
#[tokio::test]
async fn concurrent_start_watching_exactly_one_succeeds() {
    // Spawn 16 concurrent start_watching calls. Exactly 1 must return Ok,
    // 15 must return Err("Already watching"). After one iteration, the
    // watcher must be in is_watching()==true and have exactly one internal
    // notify handle (probed via internal counter).
}

#[tokio::test]
async fn claim_unwinds_on_setup_failure() {
    // Inject a path that notify cannot watch (e.g. a deleted directory).
    // start_watching must return Err AND leave is_watching()==false so
    // that a retry succeeds.
}

#[tokio::test(flavor = "multi_thread")]
async fn burst_events_do_not_block_notifier() {
    // Hammer the watched path with N > channel_capacity writes in < debounce
    // window. Assert:
    //   a) forwarder thread never blocks (check via a timeout on a sentinel
    //      event fired last),
    //   b) dropped_events() reflects the overflow count,
    //   c) at least one event still reaches the callback.
}
```

---

### #313 — with_hot_reload(true) does not apply file changes

**Root cause:** `ConfigBuilder::build` installs the watcher and calls
`config.start_watching()`, but the callback in
`lib.rs::with_hot_reload(...)` helper only logs. There is no wiring from
watcher event → `Config::reload()` on the hot-reload branch. The only path
that actually calls `Config::reload` is `with_auto_reload_interval`.

**Fix strategy:**

The reload must trigger from the watcher event stream. This requires deciding
*where* the reload happens and *what gets swapped*.

**Decision:** reload in place — `Config::data: Arc<RwLock<serde_json::Value>>`
is already the swap point. `Config::reload` builds a fresh `merged_data` and
writes it under `data.write().await`. Keep that.

**Do not switch to `ArcSwap`.** `ArcSwap` would be an improvement for
lock-free reads but changes the public contract (`Config::get` currently
holds the RwLock read guard for the duration of deserialization). Switching
the swap primitive is out of scope for a lifecycle bugfix — it is a separate
proposal to make on its own merits.

**Architectural change:** `ConfigBuilder` becomes responsible for wiring
watcher events to `Config::reload`. The current flow is:

```
builder.build() ─┬─> Config { data, watcher, ... }
                 └─> if hot_reload { config.start_watching() }   // just starts
```

The fixed flow is:

```
builder.build() ─┬─> Config { data, watcher, ... }
                 └─> if hot_reload {
                         // Watcher uses a callback supplied by the BUILDER,
                         // not by the lib.rs helper. The callback fires a
                         // dedicated reload channel; a reload task owned by
                         // the Config drains it and calls Config::reload().
                         config.start_hot_reload_pipeline()
                     }
```

**API/code shape:**

```rust
// crates/config/src/core/config.rs
impl Config {
    /// Wire watcher events to Config::reload through a debounced internal
    /// channel. Called from the builder when hot_reload is true.
    ///
    /// Ownership:
    /// - Watcher callback writes to reload_tx (non-blocking try_send).
    /// - A spawned reload task owns reload_rx, debounces coalesced events
    ///   over `RELOAD_COALESCE`, and calls self.reload().
    /// - Task exits when `self.cancel_token` fires (via Config::drop).
    pub(crate) async fn start_hot_reload_pipeline(
        self: Arc<Self>,
    ) -> ConfigResult<()> {
        const RELOAD_COALESCE: Duration = Duration::from_millis(250);

        // Watcher side: builder installed a watcher whose callback sends
        // `ReloadTrigger` values into this channel. The channel was created
        // in the builder and one end is already inside the watcher closure.
        let mut reload_rx: mpsc::Receiver<ReloadTrigger> = self
            .reload_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ConfigError::internal("reload pipeline already started"))?;

        // Now start the watcher itself (observes `cancel` for shutdown).
        if let Some(watcher) = &self.watcher {
            watcher
                .start_watching(&self.sources, self.cancel_token.clone())
                .await?;
        }

        // Spawn debounced reloader. Holds `Arc<Config>` (weak) so drop
        // semantics still fire. The reload task exits on cancel.
        let weak = Arc::downgrade(&self);
        let cancel = self.cancel_token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    maybe = reload_rx.recv() => {
                        match maybe {
                            None => break,
                            Some(_first_trigger) => {
                                // Coalesce: drain any additional triggers
                                // that arrive within RELOAD_COALESCE.
                                let deadline = tokio::time::sleep(RELOAD_COALESCE);
                                tokio::pin!(deadline);
                                loop {
                                    tokio::select! {
                                        biased;
                                        _ = cancel.cancelled() => return,
                                        _ = &mut deadline => break,
                                        trig = reload_rx.recv() => {
                                            if trig.is_none() { return; }
                                        }
                                    }
                                }
                                let Some(cfg) = weak.upgrade() else { return };
                                if let Err(e) = cfg.reload().await {
                                    nebula_log::warn!(error = %e, "hot reload failed");
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }
}

/// Internal signal passed from the watcher callback into the reload task.
/// Carries only the source that triggered the reload — the task calls
/// `Config::reload` which re-loads ALL sources.
#[derive(Debug)]
pub(crate) struct ReloadTrigger {
    pub(crate) source: ConfigSource,
    pub(crate) path: Option<PathBuf>,
}
```

```rust
// crates/config/src/core/builder.rs
// Inside ConfigBuilder::build, after creating `config`:
if self.hot_reload {
    // Builder creates the reload channel and installs a FileWatcher whose
    // callback forwards into it. Watcher is stored on the Config.
    let (reload_tx, reload_rx) = mpsc::channel::<ReloadTrigger>(64);

    let watcher: Arc<dyn ConfigWatcher> = self.watcher.unwrap_or_else(|| {
        Arc::new(FileWatcher::new(move |event: ConfigWatchEvent| {
            let trigger = ReloadTrigger {
                source: event.source,
                path: event.path,
            };
            // Non-blocking; if the reload task is behind, drop the trigger
            // — debounced reloader will catch up on the next event.
            let _ = reload_tx.try_send(trigger);
        }))
    });

    let config = config.with_watcher(watcher);            // internal setter
    config.install_reload_rx(reload_rx).await;            // stash rx for pipeline
    Arc::new(config).start_hot_reload_pipeline().await?;
    return Ok((*Arc::into_inner(...)).clone());           // or return Arc<Config>
}
```

> Note: the `Arc<Config>` gymnastics at the builder return boundary is the
> one design friction. The cleanest fix is to make `ConfigBuilder::build`
> return `Arc<Config>` when `hot_reload` is enabled. This is a minor
> breaking change but keeps the reload task alive via `Weak::upgrade`
> semantics and makes the ownership explicit. An alternative is to return
> `Config` and have the task hold a clone; `Config` is already `Clone` so
> this works without `Arc` gymnastics.

**Decision:** keep `build` returning `Config`. Inside `start_hot_reload_pipeline`,
the reload task holds a `Config` clone (cheap — shares inner `Arc<RwLock<>>`).
When the outer `Config` is dropped and `cancel_token.cancel()` fires, the
task exits via the `select!` branch. No need to introduce `Arc<Config>` at
the public API.

**Rename / semantics:** no rename needed. `with_hot_reload(true)` now
actually reloads on events. Doc comment updated to reflect this.

**Call-site impact:**

- `crates/config/src/lib.rs` — the `with_hot_reload(...)` helper that
  installs a log-only `FileWatcher` is **deleted**. It is actively
  misleading. Its only callers are documentation examples that should
  switch to `ConfigBuilder::new().with_hot_reload(true)`.
- `ConfigBuilder::build()` signature unchanged.
- `Config::start_watching()` becomes an internal detail; the public method
  stays but gets a `#[deprecated]` pointing at `with_hot_reload`. External
  callers of `config.start_watching()` do not exist outside of the builder
  today (verified via grep).

**Test strategy:**

```rust
// crates/config/tests/hot_reload_integration.rs  (new file)
#[tokio::test(flavor = "multi_thread")]
async fn hot_reload_applies_file_change_to_config_data() {
    // 1. Write initial config file: { "port": 8080 }
    // 2. Build Config with .with_hot_reload(true)
    // 3. Assert config.get::<u16>("port").await == 8080
    // 4. Rewrite file: { "port": 9090 }
    // 5. Poll config.get::<u16>("port") with a 2s timeout; assert eventually 9090
    //    (accounts for debounce + reload time).
}

#[tokio::test]
async fn hot_reload_debounces_burst() {
    // Write the same file 20 times in quick succession. Assert reload count
    // (exposed as a metric/counter) is small — coalesced — not 20.
}

#[tokio::test]
async fn hot_reload_task_exits_on_config_drop() {
    // Build a Config with hot_reload. Drop it. Assert the spawned reload
    // task's JoinHandle completes within 100ms.
}
```

---

## PR 5B — runtime stateful loop + metric skew

### #304 + #308 — cancel inside handler + persistent iteration state

**Root cause (#304):** `execute_stateful` only observes `context.cancellation`
between iterations. A handler call that blocks in I/O ignores cancel until it
naturally returns.

**Root cause (#308):** `state` is a stack local; process crash or graceful
restart resets the handler to `init_state`, losing iteration progress.

These fix together because the iteration body is the one place that (a)
needs a cancel-aware `select!` around `handler.execute`, and (b) needs a
`checkpoint(state)` call before `Continue` loops back.

**Fix strategy — #304:**

```rust
// crates/runtime/src/runtime.rs  execute_stateful loop body
let exec_fut = handler.execute(&input, &mut state, &context);
tokio::pin!(exec_fut);

let iteration_result = tokio::select! {
    biased;
    () = context.cancellation.cancelled() => {
        // Dropping the pinned future aborts the handler's in-flight work
        // at its next await point. Contract: handlers whose mid-await
        // state cannot safely be dropped must document that and guard
        // critical sections internally.
        return Err(ActionError::Cancelled);
    }
    res = &mut exec_fut => res,
};
```

Document the cancel-on-drop contract on `StatefulHandler::execute`
(`crates/action/src/stateful.rs`). This is a doc change only; no trait
signature changes.

**Fix strategy — #308:** minimal viable checkpoint.

The goal is "process restart resumes from the last completed iteration",
not "mid-iteration rollback". Iteration boundary is the atomic unit.

What to persist:

- `state: serde_json::Value` — already JSON at the `StatefulHandler` level.
- `iteration: u32` — counter for the `MAX_ITERATIONS` cap and for observability.
- `key: (ExecutionId, NodeId, u32 attempt)` — scopes the checkpoint to the
  attempt that owns it. Engine already tracks attempt numbers.

Where to persist: `ExecutionRepo`. New trait methods:

```rust
// crates/storage/src/execution_repo.rs  trait ExecutionRepo additions
/// Persist stateful iteration checkpoint.
async fn save_stateful_checkpoint(
    &self,
    execution_id: ExecutionId,
    node_id: NodeId,
    attempt: u32,
    iteration: u32,
    state: serde_json::Value,
) -> Result<(), ExecutionRepoError>;

/// Load latest stateful checkpoint for a (execution, node, attempt).
/// Returns `None` if no checkpoint exists yet.
async fn load_stateful_checkpoint(
    &self,
    execution_id: ExecutionId,
    node_id: NodeId,
    attempt: u32,
) -> Result<Option<StatefulCheckpoint>, ExecutionRepoError>;

/// Delete checkpoint after the stateful node reaches `Break`.
async fn delete_stateful_checkpoint(
    &self,
    execution_id: ExecutionId,
    node_id: NodeId,
    attempt: u32,
) -> Result<(), ExecutionRepoError>;
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatefulCheckpoint {
    pub iteration: u32,
    pub state: serde_json::Value,
}
```

Runtime signature change — `execute_stateful` takes an optional resume hook:

```rust
// crates/runtime/src/runtime.rs
/// Hook surface the engine provides so that `execute_stateful` can
/// checkpoint iteration state without the runtime depending on
/// `nebula-storage` directly.
#[async_trait]
pub trait StatefulCheckpointSink: Send + Sync {
    /// Return the last persisted checkpoint for the (execution, node,
    /// attempt) this runtime call serves, or None to start fresh.
    async fn load(&self) -> Result<Option<StatefulCheckpoint>, ActionError>;

    /// Persist the given state + iteration. Called on every successful
    /// `Continue` before the loop sleeps and recurses.
    async fn save(&self, cp: &StatefulCheckpoint) -> Result<(), ActionError>;

    /// Delete the checkpoint — called once on `Break`.
    async fn clear(&self) -> Result<(), ActionError>;
}

async fn execute_stateful(
    &self,
    metadata: &ActionMetadata,
    handler: Arc<dyn StatefulHandler>,
    input: serde_json::Value,
    context: ActionContext,
    checkpoint: Option<Arc<dyn StatefulCheckpointSink>>,
) -> Result<ActionResult<serde_json::Value>, ActionError> {
    // 1. If checkpoint.load() returns Some(cp), use cp.state as starting
    //    state and cp.iteration as starting counter. Otherwise call
    //    handler.init_state().
    // 2. Iteration loop with cancel-aware select! (#304 fix).
    // 3. On Continue: checkpoint.save(&StatefulCheckpoint { iteration, state }).
    // 4. On Break: checkpoint.clear().
    // 5. On Error: leave the checkpoint in place — a retry can resume,
    //    and the engine's attempt counter moves forward (new attempt gets
    //    a new checkpoint row, old one is garbage-collected by the engine
    //    when it transitions the node to failed).
}
```

**Call-site impact:**

- `ActionRuntime::run_handler` — call site passes the checkpoint sink. The
  engine wires it when constructing `ActionContext` or via a side channel.
  Simplest shape: extend `ActionRuntime::execute_action_versioned` to take
  an `Option<Arc<dyn StatefulCheckpointSink>>`.
- `WorkflowEngine::NodeTask` — builds the checkpoint sink backed by
  `ExecutionRepo` before dispatching to the runtime for stateful handlers.
  Stateless handlers pass `None`.
- `StatefulHandler::execute` trait — **no signature change**. Docs updated
  to state the cancel-on-drop contract explicitly.
- `InMemoryExecutionRepo` — implements the three new methods (simple
  `HashMap<(ExecutionId, NodeId, u32), StatefulCheckpoint>`).
- `PostgresExecutionRepo` — stub returning `ExecutionRepoError::Backend` for
  now; Postgres implementation is out of scope. Matches existing "Postgres
  not implemented" state per `pitfalls.md`.

**Why split the sink as a trait instead of passing `ExecutionRepo` directly:**
the runtime crate does not depend on `nebula-storage` today and should not
start. The sink trait keeps the dependency edge one-way: engine/storage →
runtime.

**Test strategy:**

```rust
// #304 regression
#[tokio::test(start_paused = true)]
async fn execute_stateful_aborts_handler_on_cancel() {
    // Handler that awaits a tokio::time::sleep(1 hour) inside execute().
    // Spawn execute_stateful on a task, cancel after 10ms, assert the
    // future returns Err(Cancelled) within 100ms, not 1 hour.
}

// #308 regression
#[tokio::test]
async fn execute_stateful_checkpoints_each_iteration() {
    // CounterAction that counts to 5 with Continue; fake checkpoint sink
    // records every save(). Assert 4 saves were recorded (one per Continue)
    // and one clear() at the end.
}

#[tokio::test]
async fn execute_stateful_resumes_from_checkpoint() {
    // Seed sink with StatefulCheckpoint { iteration: 3, state: {count:3} }.
    // Run execute_stateful; assert handler is called with count=3 on its
    // first visible iteration and counts to 5 (2 more calls, not 5).
}

#[tokio::test]
async fn execute_stateful_checkpoint_survives_mid_handler_error() {
    // Handler mutates state to count=2 then returns Retryable.
    // Assert sink.save was called with count=2 BEFORE the error propagated
    // (relies on StatefulActionAdapter's checkpoint-on-err invariant which
    //  already ships — this test pins the behavior end-to-end).
}
```

---

### #305 — duration histogram skew on early rejection

**Root cause:** `run_handler` records `duration_hist.observe(elapsed)` on
every return path, including the arms that reject dispatch without ever
calling a handler (Trigger / Resource / Agent / unknown variant). Those
arms add near-zero samples to the histogram, skewing p50/p99 downward
under a mis-routed or adversarial workload.

**Early-rejection paths — exact list:**

1. `ActionHandler::Trigger` → `TriggerNotExecutable`
2. `ActionHandler::Resource` → `ResourceNotExecutable`
3. `ActionHandler::Agent` → `AgentNotSupportedYet`
4. `_` (unknown `non_exhaustive` variant) → `Internal`

In addition, the **pre-dispatch lookup failures** in
`execute_action_versioned` / `dispatch` (`ActionNotFound`, version mismatch)
happen *before* `run_handler` is called and do not hit the histogram today,
so they are already outside the measured window. Good.

**Fix strategy:** split the counter and restrict the histogram to the
dispatched path. The histogram only observes real handler executions
(happy path + handler-returned error); the rejection branches increment a
separate labeled counter.

**API/code shape:**

```rust
// crates/runtime/src/metrics.rs (or wherever NEBULA_ACTION_* lives)
pub const NEBULA_ACTION_EXECUTIONS_TOTAL: &str = "nebula_action_executions_total";
pub const NEBULA_ACTION_FAILURES_TOTAL: &str = "nebula_action_failures_total";
pub const NEBULA_ACTION_DURATION_SECONDS: &str = "nebula_action_duration_seconds";

// NEW — rejection surface, separate counter.
pub const NEBULA_ACTION_DISPATCH_REJECTED_TOTAL: &str =
    "nebula_action_dispatch_rejected_total";

/// Reason labels for NEBULA_ACTION_DISPATCH_REJECTED_TOTAL.
pub mod dispatch_reject_reason {
    pub const TRIGGER_NOT_EXECUTABLE: &str = "trigger_not_executable";
    pub const RESOURCE_NOT_EXECUTABLE: &str = "resource_not_executable";
    pub const AGENT_NOT_SUPPORTED: &str = "agent_not_supported";
    pub const UNKNOWN_VARIANT: &str = "unknown_variant";
}
```

```rust
// crates/runtime/src/runtime.rs  run_handler
async fn run_handler(&self, /* ... */) -> Result<_, RuntimeError> {
    // Per-rejection path: increment the rejection counter ONLY. Do not
    // touch the execution counter, duration histogram, or failure counter
    // — this dispatch never reached a handler.
    let reject = |reason: &str, err: RuntimeError| -> Result<_, RuntimeError> {
        self.metrics
            .counter_with_labels(NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, &[("reason", reason)])
            .inc();
        Err(err)
    };

    // `started` is created only AFTER the match commits to a dispatched
    // variant — not before, so rejection paths do not even allocate the
    // Instant.
    let result: Result<ActionResult<serde_json::Value>, ActionError> = match handler {
        ActionHandler::Stateless(h) => {
            let started = Instant::now();
            let r = self.execute_stateless(&metadata, h, input, context).await;
            self.observe_dispatched(action_key, started, &r);
            r
        }
        ActionHandler::Stateful(h) => {
            let started = Instant::now();
            let r = self.execute_stateful(&metadata, h, input, context, cp).await;
            self.observe_dispatched(action_key, started, &r);
            r
        }
        ActionHandler::Trigger(_) => return reject(
            dispatch_reject_reason::TRIGGER_NOT_EXECUTABLE,
            RuntimeError::TriggerNotExecutable { key: action_key.to_owned() },
        ),
        ActionHandler::Resource(_) => return reject(
            dispatch_reject_reason::RESOURCE_NOT_EXECUTABLE,
            RuntimeError::ResourceNotExecutable { key: action_key.to_owned() },
        ),
        ActionHandler::Agent(_) => return reject(
            dispatch_reject_reason::AGENT_NOT_SUPPORTED,
            RuntimeError::AgentNotSupportedYet { key: action_key.to_owned() },
        ),
        _ => return reject(
            dispatch_reject_reason::UNKNOWN_VARIANT,
            RuntimeError::Internal(format!(
                "unknown ActionHandler variant for action '{action_key}'"
            )),
        ),
    };

    // ... enforce_data_limit on Ok(...) ...
}

fn observe_dispatched(
    &self,
    action_key: &str,
    started: Instant,
    result: &Result<ActionResult<serde_json::Value>, ActionError>,
) {
    let elapsed = started.elapsed();
    self.metrics
        .histogram(NEBULA_ACTION_DURATION_SECONDS)
        .observe(elapsed.as_secs_f64());
    self.metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).inc();
    if result.is_err() {
        self.metrics.counter(NEBULA_ACTION_FAILURES_TOTAL).inc();
    }
}
```

**Call-site impact:** no public runtime API changes. Dashboards that alerted
on `NEBULA_ACTION_EXECUTIONS_TOTAL` now see only real executions (throughput
graph becomes accurate). A new alert should be added on
`NEBULA_ACTION_DISPATCH_REJECTED_TOTAL` — surfaces the mis-routing / attack
signal that currently hides inside the execution count.

**Test strategy:**

```rust
#[tokio::test]
async fn trigger_rejection_does_not_observe_histogram() {
    // Register a handler with ActionHandler::Trigger(_). Call run_handler.
    // Assert:
    //   NEBULA_ACTION_DURATION_SECONDS.count == 0
    //   NEBULA_ACTION_EXECUTIONS_TOTAL.count == 0
    //   NEBULA_ACTION_DISPATCH_REJECTED_TOTAL{reason=trigger_not_executable} == 1
}

#[tokio::test]
async fn dispatched_stateless_observes_histogram_and_counter() {
    // Register a trivial stateless handler. Call run_handler.
    // Assert histogram count == 1, executions_total == 1,
    // rejected_total == 0.
}
```

---

## PR 5C — storage lease TTL + idempotency dead code

### #317 — InMemoryExecutionRepo ignores lease TTL

**Root cause:** `leases: HashMap<ExecutionId, String>` stores only the
holder; `acquire_lease(..., _ttl)` ignores TTL; `renew_lease` only checks
holder equality.

**Fix strategy:** store `(holder, expires_at: Instant)` in the map. `acquire`
and `renew` check `Instant::now() > expires_at` and fall back to acquire on
expiration.

**API/code shape:**

```rust
// crates/storage/src/execution_repo.rs
type LeaseEntry = (String, Instant);

#[derive(Default)]
pub struct InMemoryExecutionRepo {
    // ...existing fields...
    leases: Arc<RwLock<HashMap<ExecutionId, LeaseEntry>>>,
}

#[async_trait]
impl ExecutionRepo for InMemoryExecutionRepo {
    async fn acquire_lease(
        &self,
        id: ExecutionId,
        holder: String,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let now = Instant::now();
        match leases.get(&id) {
            Some((_, expires)) if *expires > now => return Ok(false), // still held
            _ => {}
        }
        leases.insert(id, (holder, now + ttl));
        Ok(true)
    }

    async fn renew_lease(
        &self,
        id: ExecutionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        let now = Instant::now();
        match leases.get_mut(&id) {
            Some((current, expires)) if current == holder && *expires > now => {
                *expires = now + ttl;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release_lease(
        &self,
        id: ExecutionId,
        holder: &str,
    ) -> Result<bool, ExecutionRepoError> {
        let mut leases = self.leases.write().await;
        if let Some((current, _)) = leases.get(&id) {
            if current == holder {
                leases.remove(&id);
                return Ok(true);
            }
        }
        Ok(false)
    }
}
```

Note: `Instant` is monotonic, not `Serialize` — fine for in-memory repo.
Postgres uses SQL `now() + interval` and is already correct.

**Call-site impact:** none. `ExecutionRepo` trait signature unchanged.

**Test strategy:**

```rust
#[tokio::test(start_paused = true)]
async fn in_memory_lease_expires_after_ttl() {
    let repo = InMemoryExecutionRepo::new();
    let id = ExecutionId::new();
    assert!(repo.acquire_lease(id, "A".into(), Duration::from_secs(5)).await.unwrap());
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(repo.acquire_lease(id, "B".into(), Duration::from_secs(5)).await.unwrap());
    // A's stale lease does not block B.
}

#[tokio::test(start_paused = true)]
async fn in_memory_lease_renew_extends_expiry() {
    // Acquire, advance 3s, renew, advance 3s, try to steal — must fail
    // because renew pushed expiry to 3+5 = 8s total.
}

#[tokio::test(start_paused = true)]
async fn in_memory_lease_renew_rejects_wrong_holder() {
    // A acquires. B attempts renew — returns Ok(false), no expiry change.
}
```

---

### #303 — IdempotencyManager dead code

**Root cause:** `IdempotencyManager { seen: HashSet<String> }` is a latent
duplicate of `ExecutionRepo::{check_idempotency, mark_idempotent}`. Nothing
in the durable engine path consults it — verified via grep:
`IdempotencyManager` is only referenced inside its own module and tests.
Keeping the struct is a future-refactor footgun (see
`.project/context/decisions.md` class of traps — two sources of truth with
the same API shape).

**Fix strategy:** **delete `IdempotencyManager` entirely**. Keep
`IdempotencyKey` (used by the engine to build the key passed to the repo).

**API/code shape:**

```rust
// crates/execution/src/idempotency.rs  — AFTER
//! Idempotency key generation. Deduplication is owned by `ExecutionRepo`.

use nebula_core::{ExecutionId, NodeId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    #[must_use]
    pub fn generate(execution_id: ExecutionId, node_id: NodeId, attempt: u32) -> Self {
        Self(format!("{execution_id}:{node_id}:{attempt}"))
    }
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

Remove from `crates/execution/src/lib.rs`:

```rust
pub use idempotency::{IdempotencyKey, IdempotencyManager};  // BEFORE
pub use idempotency::IdempotencyKey;                        // AFTER
```

**Call-site impact:** the only non-test references to `IdempotencyManager`
are the re-export in `lib.rs` and the doc comment at the top of the crate.
Both removed. No call sites outside `execution` crate tests.

**Test strategy:**

- Delete `idempotency.rs` tests that exercised `check_and_mark`, `is_seen`,
  `clear`, `len`, `is_empty`. Keep the tests for `IdempotencyKey::generate`
  and `Display`.
- Add a regression test in `crates/engine/src/engine.rs` (or integration)
  confirming the idempotency flow still routes through `ExecutionRepo`:

```rust
#[tokio::test]
async fn idempotency_deduplication_routes_through_repo() {
    // Build an engine with InMemoryExecutionRepo. Run a node twice with
    // the same (execution, node, attempt). Second attempt must short-circuit
    // via repo.check_idempotency — probe this by spying on the repo or by
    // asserting the handler's call counter is 1 even though dispatch was
    // called twice.
}
```

---

## PR 5D — engine credential refresh fail-open

### #306 — proactive credential refresh failure ignored

**Root cause:** `NodeTask::run` fires the `credential_refresh` hook before
`execute_action_versioned`. On any error, it logs a `WARN` and proceeds,
almost guaranteeing a downstream opaque auth failure. There is no
classification, no backpressure, no error propagation, and no cancellation
awareness.

**Architect call on semantics:** The refresh hook's role is "rotate a
credential that is about to expire so the next call has a fresh token".
Three possible outcomes:

1. **Refresh not needed** (credential still valid) — hook returns Ok, no-op.
2. **Refresh succeeded** — hook returns Ok, credential cache updated.
3. **Refresh failed** — the hook cannot rotate. Either the store is broken,
   the token is already expired, or the refresh endpoint is rate-limited.

The current code treats (3) as "best effort, proceed anyway". The correct
default treatment is **"surface as a typed error that `ErrorStrategy` can
act on"**. Rationale:

- A broken credential store is a real failure mode and operators need it
  visible as an execution-level signal, not buried in a WARN.
- `ErrorStrategy` already has the vocabulary — retry, dead-letter, fail,
  skip. Refresh failures should route through the same policy surface as
  any other action failure.
- "Proceed with potentially stale credential" leaks into N downstream
  opaque auth errors, each with its own retry/backoff. Failing once at the
  refresh step is strictly less noise.

**Fix strategy:**

1. **New error variant.** Add
   `ActionError::CredentialRefreshFailed { source: Arc<dyn Error + Send + Sync>, action_key: ActionKey }`.
   The engine surfaces this instead of logging a WARN.

2. **ErrorStrategy classification.** `CredentialRefreshFailed` is by default
   classified as `Retryable` — the store may be transiently down, a retry
   in 100ms may succeed, and the existing retry/backoff wiring handles it.
   Fatal classification can be opted into per-action via metadata if the
   operator wants fail-fast.

3. **Cancel-aware refresh.** Wrap `refresh_fn(&action_key)` in
   `tokio::select!` against `self.cancel.cancelled()`. Shutdown cannot wait
   on a dying store.

4. **Circuit break at the engine level (deferred).** The issue suggests a
   circuit breaker on the refresh path to prevent log amplification under
   sustained store failure. This is a good idea, but `nebula-resilience`
   already ships `CircuitBreaker`. **Scope call:** do NOT add a circuit
   breaker in Batch 5D. Ship the typed error path first; operators can
   wrap `refresh_fn` in a `CircuitBreaker` themselves at the point where
   they construct the hook. A follow-up issue can move the breaker
   inside `NodeTask` if usage patterns show it is always needed.

**API/code shape:**

```rust
// crates/action/src/error.rs  (ActionError enum)
#[non_exhaustive]
pub enum ActionError {
    // ...existing variants...

    /// Proactive credential refresh failed before action dispatch.
    ///
    /// Classified as retryable by default — the credential store may be
    /// transiently unavailable. Operators can configure an action to
    /// treat this as fatal via metadata.
    #[error("credential refresh failed for action '{action_key}': {source}")]
    CredentialRefreshFailed {
        action_key: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl ActionError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Retryable { .. } | Self::CredentialRefreshFailed { .. }
        ) || /* existing conditions */
    }
}
```

```rust
// crates/engine/src/engine.rs  NodeTask::run  (around line 1539)
if let Some(ref refresh_fn) = self.credential_refresh {
    let refresh_fut = (refresh_fn)(&self.action_key);
    tokio::pin!(refresh_fut);
    let refresh_result = tokio::select! {
        biased;
        () = self.cancel.cancelled() => {
            return (self.node_id, Err(EngineError::Cancelled));
        }
        res = &mut refresh_fut => res,
    };

    if let Err(e) = refresh_result {
        // Stop fire-and-log. Surface as a typed ActionError so the engine's
        // existing ErrorStrategy path runs (retry, dead-letter, fail).
        let action_err = ActionError::CredentialRefreshFailed {
            action_key: self.action_key.to_string(),
            source: Box::new(e),
        };
        return (self.node_id, Err(EngineError::ActionFailed(action_err)));
    }
}
```

**Call-site impact:**

- `ActionError` gains a new `#[non_exhaustive]` variant — all exhaustive
  matches on `ActionError` need a new arm. Verified: `ActionError` already
  has `#[non_exhaustive]`, so this is a minor-version bump at most.
- The engine's `ErrorStrategy` classifier already handles `is_retryable()`
  and `is_fatal()`; new variant defaults to retryable.
- Public `with_credential_refresh` hook signature unchanged — still returns
  `Result<(), E>`; engine now propagates the `E` as a boxed error into the
  typed variant.

**Test strategy:**

```rust
#[tokio::test]
async fn credential_refresh_failure_surfaces_as_typed_error() {
    // Register an action with refresh_fn returning Err("store down").
    // Dispatch the node. Assert the resulting error is
    // ActionError::CredentialRefreshFailed and its source string contains
    // "store down".
}

#[tokio::test]
async fn credential_refresh_retries_via_error_strategy() {
    // refresh_fn returns Err on first call, Ok on second. Configure node
    // with ErrorStrategy::Retry(max=1). Assert the action is dispatched
    // exactly once with the fresh credential.
}

#[tokio::test]
async fn credential_refresh_respects_cancellation() {
    // refresh_fn awaits a tokio::time::sleep(10s). Cancel after 50ms.
    // Assert NodeTask::run returns EngineError::Cancelled within 200ms.
}
```

---

## PR 5E — resource release queue saturation

### #309 — ReleaseQueue drops cleanup tasks silently

**Root cause:** `ReleaseQueue::submit` uses `try_send` to the primary
worker; on `Full` it falls back to a secondary channel; on double-`Full`
it increments `dropped_count` and returns — the cleanup task is gone and
the resource leaks.

**Fix strategy:** bounded-wait with explicit backpressure, plus
observability. The options listed in the issue (A bounded retry, B blocking
send with timeout, C dead-letter + sweeper) reduce to one in practice:
**B with a short timeout** is the smallest change that prevents loss and is
compatible with `submit`'s non-async signature.

But `submit` is currently sync — it cannot `.await`. Two shapes:

**Shape 1 (minimal):** keep `submit` sync, but on double-`Full` do not drop.
Spawn a tiny rescue task that awaits `primary.send(factory)` (blocking send
that waits for capacity) with a bounded timeout and cancel observation.
The rescue task owns the factory until it either lands on a worker or the
engine is shutting down.

**Shape 2 (explicit):** add `submit_async` that awaits `send_timeout`
directly. Callers that want sync semantics use `submit`; callers that want
to guarantee delivery use `submit_async`.

**Decision:** Shape 1. Callers of `submit` today are fire-and-forget from
drop paths that cannot easily be made `async`. Forcing them to choose
between `submit` and `submit_async` regresses ergonomics without adding
clarity. The rescue task is a local implementation detail.

**API/code shape:**

```rust
// crates/resource/src/release_queue.rs

/// Maximum time a rescue task waits for a worker channel to free up.
///
/// If no worker has capacity within this window, the task is recorded
/// as truly dropped via `dropped_count` — an explicit, metric-observable
/// loss, not a silent one.
const RESCUE_TIMEOUT: Duration = Duration::from_secs(30);

impl ReleaseQueue {
    pub fn submit(&self, factory: impl FnOnce() -> ReleaseTask + Send + 'static) {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let factory: TaskFactory = Box::new(factory);

        // Primary try.
        match self.senders[idx].try_send(factory) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(factory)) => {
                // Fallback try.
                let count = self.fallback_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_power_of_two() {
                    tracing::warn!(
                        fallback_tasks = count,
                        "release queue primary channels full, using fallback"
                    );
                }
                match self.fallback_tx.try_send(factory) {
                    Ok(()) => return,
                    Err(mpsc::error::TrySendError::Full(factory)) => {
                        // Previously: drop silently. Now: rescue.
                        self.spawn_rescue(factory);
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        tracing::warn!("release queue fallback channel closed");
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("release queue primary channel closed");
            }
        }
    }

    /// Non-blocking rescue path for the double-full case.
    ///
    /// Spawns a short-lived task that awaits capacity on the fallback
    /// channel (blocking send) for up to `RESCUE_TIMEOUT`. If the queue
    /// is cancelled or the timeout expires, the task is recorded as
    /// dropped — this is the only path that counts toward `dropped_count`,
    /// and it is bounded and observable.
    fn spawn_rescue(&self, factory: TaskFactory) {
        let fallback_tx = self.fallback_tx.clone();
        let cancel = self.cancel.clone();
        let dropped = Arc::clone(/* wrap dropped_count in Arc if not already */);
        let rescue_in_flight = /* increment a new rescue counter */;

        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    // Shutdown — account as dropped and exit. The worker
                    // drain loop cannot rescue us now.
                    record_drop(&dropped, "shutdown");
                }
                res = tokio::time::timeout(RESCUE_TIMEOUT, fallback_tx.send(factory)) => {
                    match res {
                        Ok(Ok(())) => { /* rescued; no drop */ }
                        Ok(Err(_closed)) => record_drop(&dropped, "channel_closed"),
                        Err(_elapsed) => record_drop(&dropped, "timeout"),
                    }
                }
            }
        });
    }
}

fn record_drop(counter: &Arc<AtomicUsize>, reason: &'static str) {
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_power_of_two() {
        tracing::error!(
            dropped_tasks = n,
            reason = reason,
            "release queue rescue failed — resource may leak"
        );
    }
}
```

**Operational metric:** `dropped_count` is already on the struct. Expose it
via a public getter `pub fn dropped_count(&self) -> usize`, plus a new
`pub fn rescued_count(&self) -> usize` so ops can see "we entered rescue
path N times, lost M". Wire these as Prometheus gauges in
`crates/metrics`.

**Call-site impact:** none. `ReleaseQueue::submit` signature unchanged.
Caller semantics are strictly better (fewer silent drops).

**Shutdown interaction:** the rescue task observes `self.cancel`. During
graceful shutdown, rescue tasks in flight short-circuit to "dropped
(shutdown)" rather than holding shutdown open. `ReleaseQueueHandle::shutdown`
awaits primary + fallback workers but does NOT await rescue tasks — they
are fire-and-forget because their whole purpose is to survive without a
caller handle. The rescue path's timeout (30s) caps their total lifetime.

**Test strategy:**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn saturated_queue_rescues_via_background_task() {
    // 1 worker, channel 256, fallback 4096. Submit 10_000 slow tasks
    // (each one sleeps 50ms) — primary and fallback will both saturate.
    // Assert:
    //   a) every task eventually completes (counter == 10_000),
    //   b) queue.dropped_count() == 0,
    //   c) queue.rescued_count() > 0 (we exercised the rescue path).
}

#[tokio::test(start_paused = true)]
async fn rescue_timeout_records_drop() {
    // Saturate queues. Fill the fallback such that no worker will drain
    // within RESCUE_TIMEOUT. Cancel the queue. Assert dropped_count > 0
    // and the drop reason metric reports "shutdown" (or "timeout" for
    // the non-cancel variant).
}

#[tokio::test]
async fn rescue_observes_cancellation_immediately() {
    // Saturate + cancel. Rescue tasks must not hold shutdown — assert
    // ReleaseQueue::shutdown(handle) completes within 200ms.
}
```

---

## Risks and gotchas

### R1 — PR 5A depends on PR #363 being merged first

PR #363 (`fix(config, credential): batch 3b watcher drop ...`) changes the
`ConfigWatcher::start_watching` trait signature to take a `CancellationToken`.
All of 5A's code assumes that signature. If 5A is rebased onto `main` before
#363 merges, it will collide at the trait definition and both impls
(`FileWatcher`, `PollingWatcher`, `NoopWatcher`) will need to carry the
signature change a second time.

**Mitigation:** merge order is PR #363 → PR 5A. If #363 stalls, 5A can
either block on it or absorb the trait-signature change itself — but that
makes 5A the authoritative carrier of #363's work and the two should be
merged under a single PR in that case.

### R2 — #308 persistence changes `ExecutionRepo` trait

Three new methods on `ExecutionRepo`: `save_stateful_checkpoint`,
`load_stateful_checkpoint`, `delete_stateful_checkpoint`. This cascades to:

- `InMemoryExecutionRepo` (straightforward).
- `PostgresExecutionRepo` (currently not implemented per pitfalls.md, but
  the trait impl has to compile — ship as `unimplemented!` or
  `ExecutionRepoError::Backend("not implemented")`).
- Any test doubles / spy repos across the codebase.

The runtime side uses a `StatefulCheckpointSink` trait to avoid dragging
`nebula-storage` into `nebula-runtime`. The engine is the glue layer that
implements the sink backed by the repo.

**Non-obvious gotcha:** `StatefulHandler::init_state` is idempotent by
contract (pure function), so resuming a handler whose state could not be
deserialized safely falls back to `init_state` only if the handler did not
define `migrate_state`. The resume path in `execute_stateful` must call
`migrate_state(checkpoint.state.clone())` when deserialization fails, then
`init_state()` as the final fallback. If `init_state` is used as a fallback
after a checkpoint existed, we are **silently losing iteration progress** —
this MUST be logged at WARN with action_key, execution_id, node_id so the
loss is visible.

---

## Scope guards

These are explicitly **not** in Batch 5:

- Circuit breaker inside `NodeTask` refresh path (operator-wraps instead).
- `ArcSwap` for `Config::data` (separate proposal, different trade-off).
- Postgres implementation of stateful checkpoint methods (Postgres storage
  is broadly stubbed per pitfalls).
- Reshaping `IdempotencyManager` into a write-through cache in front of the
  repo (the issue lists this as option 2; Batch 5 chooses option 1 —
  deletion — because there is no current caller that benefits from the
  cache).
- Histogram labeling by `action_key` for #305 (separate observability
  proposal).
- `submit_async` on `ReleaseQueue` (Batch 5E ships the rescue path; a
  separate API surface is a different change).

---

## Summary

Five PRs, ~10 issues, no cross-PR coupling except PR 5A → PR #363 merge
ordering. Each PR is independently reviewable, testable, revertable.

Risk is concentrated in:
1. PR 5B (#308) — touches `ExecutionRepo` trait, cascades to every impl.
2. PR 5A (#313) — reshapes how `ConfigBuilder::build` wires hot reload,
   deletes the misleading `lib.rs::with_hot_reload` helper.

Everything else is local.
