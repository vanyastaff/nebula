# nebula-resource Improvements Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix correctness bugs, wire existing scaffolding into the acquire/release paths, and improve API ergonomics — everything except Credential-related work.

**Architecture:** Three waves — correctness fixes first (independent, parallelizable), then wiring existing types into live paths, then API polish. Each wave has a checkpoint where all tests must pass before moving on.

**Tech Stack:** Rust 1.93+, tokio, arc-swap, dashmap, tokio-util

---

## Wave 1 — Correctness Fixes

### Task 1: ReleaseQueue unbounded fallback

The bounded fallback channel (1024) silently drops release tasks when full. For Pool/Transport/Exclusive topologies, the dropped task holds a semaphore permit — permanently leaked, pool degrades irreversibly.

**Files:**
- Modify: `crates/resource/src/release_queue.rs`

**Step 1: Write failing test — overflow drops are detected**

Add to `release_queue.rs` `mod tests`:

```rust
#[tokio::test]
async fn fallback_channel_never_drops_tasks() {
    // Create queue with 1 worker, tiny primary buffer.
    // Submit more tasks than primary + old fallback capacity.
    // All tasks must complete — none dropped.
    let (queue, handle) = ReleaseQueue::new(1);
    let counter = Arc::new(AtomicU32::new(0));

    // Pause workers by filling primary with a slow task.
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let b = barrier.clone();
    queue.submit(move || {
        Box::pin(async move {
            b.wait().await;
        })
    });

    // Submit 2000 tasks (exceeds old FALLBACK_BUFFER=1024).
    for _ in 0..2000 {
        let c = counter.clone();
        queue.submit(move || {
            Box::pin(async move {
                c.fetch_add(1, Ordering::Relaxed);
            })
        });
    }

    // Unblock the barrier task.
    barrier.wait().await;

    // Give workers time to drain.
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert_eq!(
        counter.load(Ordering::Relaxed),
        2000,
        "all tasks must complete — none dropped"
    );

    drop(queue);
    ReleaseQueue::shutdown(handle).await;
}
```

**Step 2: Run test to verify it fails**

```bash
rtk cargo nextest run -p nebula-resource -- fallback_channel_never_drops
```

Expected: FAIL — old bounded fallback drops tasks beyond 1024+256.

**Step 3: Change fallback to unbounded**

In `release_queue.rs`:

1. Remove `const FALLBACK_BUFFER: usize = 1024;`
2. Change `ReleaseQueue.fallback_tx` type from `mpsc::Sender<TaskFactory>` to `mpsc::UnboundedSender<TaskFactory>`
3. In `new()`: replace `mpsc::channel::<TaskFactory>(FALLBACK_BUFFER)` with `mpsc::unbounded_channel::<TaskFactory>()`
4. Add `worker_loop_unbounded` for the unbounded receiver (same logic, different recv type)
5. In `submit()`: replace `self.fallback_tx.try_send(factory)` with `self.fallback_tx.send(factory)` — unbounded send never fails for capacity reasons (only if receiver is dropped)

```rust
// release_queue.rs changes:

pub struct ReleaseQueue {
    senders: Vec<mpsc::Sender<TaskFactory>>,
    fallback_tx: mpsc::UnboundedSender<TaskFactory>,
    next: AtomicUsize,
}

// In new():
let (fallback_tx, fallback_rx) = mpsc::unbounded_channel::<TaskFactory>();
let fallback_worker = tokio::spawn(Self::worker_loop_unbounded(fallback_rx));

// In submit():
Err(mpsc::error::TrySendError::Full(factory)) => {
    if self.fallback_tx.send(factory).is_err() {
        tracing::error!(
            "release queue overflow: fallback channel closed, \
             dropping release task (may leak semaphore permit)"
        );
    }
}

// New worker loop for unbounded:
async fn worker_loop_unbounded(mut rx: mpsc::UnboundedReceiver<TaskFactory>) {
    while let Some(factory) = rx.recv().await {
        let task = factory();
        if tokio::time::timeout(TASK_EXECUTION_TIMEOUT, task)
            .await
            .is_err()
        {
            tracing::warn!(
                "release task timed out after {}s, skipping",
                TASK_EXECUTION_TIMEOUT.as_secs()
            );
        }
    }
}
```

**Step 4: Run test to verify it passes**

```bash
rtk cargo nextest run -p nebula-resource -- release_queue
```

Expected: ALL PASS including new test.

**Step 5: Commit**

```
fix(resource): use unbounded fallback channel in ReleaseQueue

Bounded fallback (1024) silently dropped tasks under sustained load,
permanently leaking semaphore permits and degrading pool capacity.
```

---

### Task 2: ResidentRuntime — Mutex on create path + graceful destroy

Two race conditions:
1. Concurrent acquires on empty cell both call `create()`, second overwrites first without `destroy()`.
2. `Arc::try_unwrap` failure leaks runtime without graceful destroy.

**Files:**
- Modify: `crates/resource/src/runtime/resident.rs`

**Step 1: Write failing test — concurrent create race**

```rust
#[tokio::test]
async fn concurrent_acquire_creates_only_once() {
    let resource = MockResident::new();
    let config = Config { recreate_on_failure: true };
    let rt = Arc::new(ResidentRuntime::<MockResident>::new(config));
    let ctx = test_ctx();

    // Launch 10 concurrent acquires on empty cell.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let rt = rt.clone();
        let r = resource.clone();
        let c = ctx.clone();
        handles.push(tokio::spawn(async move {
            rt.acquire(&r, &true, &(), &c).await
        }));
    }

    for h in handles {
        h.await.unwrap().unwrap();
    }

    // Only one create should have happened.
    assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
}
```

**Step 2: Run test to verify it fails**

```bash
rtk cargo nextest run -p nebula-resource -- concurrent_acquire_creates_only_once
```

Expected: FAIL — multiple creates due to race.

**Step 3: Add Mutex to guard create path**

```rust
use tokio::sync::Mutex;

pub struct ResidentRuntime<R: Resource> {
    cell: Cell<R::Runtime>,
    config: Config,
    create_lock: Mutex<()>,
}

impl<R: Resource> ResidentRuntime<R> {
    pub fn new(config: Config) -> Self {
        Self {
            cell: Cell::new(),
            config,
            create_lock: Mutex::new(()),
        }
    }
    // ...
}
```

In `acquire()`, restructure:

```rust
pub async fn acquire(
    &self,
    resource: &R,
    resource_config: &R::Config,
    credential: &R::Credential,
    ctx: &dyn Ctx,
) -> Result<ResourceHandle<R>, Error>
where
    R::Runtime: Into<R::Lease>,
{
    // Fast path: existing, alive instance.
    if let Some(existing) = self.cell.load() {
        if resource.is_alive_sync(&existing) {
            let lease: R::Lease = (*existing).clone().into();
            return Ok(ResourceHandle::owned(lease, R::key(), "resident"));
        }
    }

    // Slow path: need to create or recreate, under lock.
    let _guard = self.create_lock.lock().await;

    // Double-check after acquiring lock.
    if let Some(existing) = self.cell.load() {
        if resource.is_alive_sync(&existing) {
            let lease: R::Lease = (*existing).clone().into();
            return Ok(ResourceHandle::owned(lease, R::key(), "resident"));
        }

        // Not alive — destroy if configured.
        if !self.config.recreate_on_failure {
            return Err(Error::transient("resident runtime is not alive"));
        }

        // Take and schedule background destroy (don't block on try_unwrap).
        if let Some(old) = self.cell.take() {
            // Spawn best-effort destroy — works even if Arc has other holders.
            let r = resource.clone();
            tokio::spawn(async move {
                match Arc::try_unwrap(old) {
                    Ok(owned) => {
                        let _ = r.destroy(owned).await;
                    }
                    Err(arc) => {
                        // Other holders exist — wait for them via Weak.
                        // Log and let Arc drop naturally when all holders release.
                        tracing::warn!(
                            key = %R::key(),
                            strong_count = Arc::strong_count(&arc),
                            "resident runtime has active holders, \
                             destroy deferred to last Arc drop"
                        );
                    }
                }
            });
        }
    }

    // Create new runtime.
    let runtime = resource
        .create(resource_config, credential, ctx)
        .await
        .map_err(Into::into)?;

    let lease: R::Lease = runtime.clone().into();
    self.cell.store(Arc::new(runtime));

    Ok(ResourceHandle::owned(lease, R::key(), "resident"))
}
```

Note: `MockResident` needs `Clone` (already has it) and `R` needs `Clone + 'static` for the spawn. Add bound `R: Clone + 'static` to the impl block.

**Step 4: Run all resident tests**

```bash
rtk cargo nextest run -p nebula-resource -- resident
```

Expected: ALL PASS.

**Step 5: Commit**

```
fix(resource): prevent concurrent create race in ResidentRuntime

Added tokio::Mutex to guard the create-or-recreate path with
double-checked locking. Failed Arc::try_unwrap now spawns
background destroy instead of silently leaking.
```

---

### Task 3: TOCTOU fix in Manager::remove

**Files:**
- Modify: `crates/resource/src/registry.rs` (Registry::remove return type)
- Modify: `crates/resource/src/manager.rs` (Manager::remove logic)

**Step 1: Write failing test — concurrent double remove**

In `tests/basic_integration.rs` or in `manager.rs` tests:

```rust
#[tokio::test]
async fn remove_nonexistent_returns_not_found() {
    let manager = Manager::new();
    let key = resource_key!("nonexistent");
    let result = manager.remove(&key);
    assert!(matches!(result.unwrap_err().kind(), ErrorKind::NotFound));
}
```

**Step 2: Change Registry::remove to return bool**

```rust
// registry.rs
pub fn remove(&self, key: &ResourceKey) -> bool {
    let existed = self.entries.remove(key).is_some();
    if existed {
        self.type_index.retain(|_type_id, k| k != key);
    }
    existed
}
```

**Step 3: Update Manager::remove to use atomic check-and-remove**

```rust
// manager.rs
pub fn remove(&self, key: &ResourceKey) -> Result<(), Error> {
    if !self.registry.remove(key) {
        return Err(Error::not_found(key));
    }
    self.metrics.record_destroy();
    tracing::debug!(%key, "resource removed");
    Ok(())
}
```

**Step 4: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

Expected: ALL PASS.

**Step 5: Commit**

```
fix(resource): eliminate TOCTOU race in Manager::remove

Registry::remove now returns bool, Manager uses it atomically
instead of separate contains() + remove().
```

---

## Wave 1 Checkpoint

```bash
rtk cargo fmt && rtk cargo clippy -p nebula-resource -- -D warnings && rtk cargo nextest run -p nebula-resource
```

ALL PASS before proceeding.

---

## Wave 2 — Wire Existing Types

### Task 4: `#[non_exhaustive]` on public enums

Per project quality standards: "Exhaustive enums get `#[non_exhaustive]` if they may grow."

**Files:**
- Modify: `crates/resource/src/error.rs` — `ErrorKind`, `ErrorScope`
- Modify: `crates/resource/src/state.rs` — `ResourcePhase`
- Modify: `crates/resource/src/events.rs` — `ResourceEvent`
- Modify: `crates/resource/src/options.rs` — `AcquireIntent`
- Modify: `crates/resource/src/topology/pooled.rs` — `BrokenCheck`, `RecycleDecision`
- Modify: `crates/resource/src/topology/service.rs` — `TokenMode`
- Modify: `crates/resource/src/topology/daemon.rs` — `RestartPolicy`
- Modify: `crates/resource/src/recovery/gate.rs` — `GateState`

**Step 1: Add `#[non_exhaustive]` to each enum**

```rust
// error.rs
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind { ... }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorScope { ... }

// state.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourcePhase { ... }

// events.rs
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ResourceEvent { ... }

// options.rs
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AcquireIntent { ... }

// topology/pooled.rs
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BrokenCheck { ... }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecycleDecision { ... }

// topology/service.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenMode { ... }

// topology/daemon.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestartPolicy { ... }

// recovery/gate.rs
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum GateState { ... }
```

**Step 2: Add wildcard arms to all internal match statements**

Search for exhaustive matches on these enums within the crate and add `_ => unreachable!()` or meaningful fallback arms where needed. Key locations:
- `state.rs:37-46` — `ResourcePhase::Display` match
- `state.rs:25-27` — `is_accepting()`
- `state.rs:30-32` — `is_terminal()`
- `error.rs:83-87` — `is_retryable()`
- `error.rs:91-95` — `retry_after()`
- `events.rs:70-77` — `ResourceEvent::key()`
- `recovery/gate.rs` — multiple match arms in `try_begin()`

**Step 3: Run tests and fix any breakage**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 4: Commit**

```
refactor(resource): add #[non_exhaustive] to all public enums

Prevents semver breakage when adding variants to ErrorKind,
ResourcePhase, ResourceEvent, AcquireIntent, GateState, etc.
```

---

### Task 5: TopologyTag enum replacing `&'static str`

**Files:**
- Create: `crates/resource/src/topology_tag.rs`
- Modify: `crates/resource/src/handle.rs` — change topology_tag type
- Modify: `crates/resource/src/lib.rs` — add module + re-export
- Modify: `crates/resource/src/runtime/pool.rs` — use `TopologyTag::Pool`
- Modify: `crates/resource/src/runtime/resident.rs` — use `TopologyTag::Resident`
- Modify: `crates/resource/src/runtime/service.rs` — use `TopologyTag::Service`
- Modify: `crates/resource/src/runtime/transport.rs` — use `TopologyTag::Transport`
- Modify: `crates/resource/src/runtime/exclusive.rs` — use `TopologyTag::Exclusive`

**Step 1: Create TopologyTag enum**

```rust
// topology_tag.rs
//! Topology identifier tag.

use std::fmt;

/// Identifies which topology a resource handle was acquired from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TopologyTag {
    /// Pool — N interchangeable instances.
    Pool,
    /// Resident — one shared instance, clone on acquire.
    Resident,
    /// Service — long-lived runtime, short-lived tokens.
    Service,
    /// Transport — shared connection, multiplexed sessions.
    Transport,
    /// Exclusive — one caller at a time.
    Exclusive,
    /// EventSource — pull-based event subscription.
    EventSource,
    /// Daemon — background run loop.
    Daemon,
}

impl TopologyTag {
    /// Returns the tag as a static string slice.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pool => "pool",
            Self::Resident => "resident",
            Self::Service => "service",
            Self::Transport => "transport",
            Self::Exclusive => "exclusive",
            Self::EventSource => "event_source",
            Self::Daemon => "daemon",
        }
    }
}

impl fmt::Display for TopologyTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
```

**Step 2: Update ResourceHandle**

In `handle.rs`:
- Change `topology_tag: &'static str` → `topology_tag: TopologyTag`
- Change `topology_tag()` return type → `TopologyTag`
- Update `owned()`, `guarded()`, `shared()` signatures

**Step 3: Update all runtime call sites**

Replace string literals with enum variants:
- `runtime/pool.rs`: `"pool"` → `TopologyTag::Pool`
- `runtime/resident.rs`: `"resident"` → `TopologyTag::Resident`
- `runtime/service.rs`: `"service"` → `TopologyTag::Service`
- `runtime/transport.rs`: `"transport"` → `TopologyTag::Transport`
- `runtime/exclusive.rs`: `"exclusive"` → `TopologyTag::Exclusive`

**Step 4: Update tests**

In `handle.rs` tests, replace `"pool"`, `"resident"`, `"test"` with appropriate `TopologyTag` variants.
In `tests/basic_integration.rs`, update any topology_tag assertions.

**Step 5: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 6: Commit**

```
refactor(resource): replace topology_tag &str with TopologyTag enum

Type-safe topology identification instead of stringly-typed tags.
Enables exhaustive matching and prevents typos.
```

---

### Task 6: Re-export runtime types from lib.rs

**Files:**
- Modify: `crates/resource/src/lib.rs`

**Step 1: Add re-exports**

```rust
// lib.rs — add after existing re-exports:
pub use runtime::TopologyRuntime;
pub use runtime::pool::PoolRuntime;
pub use runtime::resident::ResidentRuntime;
pub use runtime::service::ServiceRuntime;
pub use runtime::transport::TransportRuntime;
pub use runtime::exclusive::ExclusiveRuntime;
pub use runtime::event_source::EventSourceRuntime;
pub use runtime::daemon::DaemonRuntime;
pub use runtime::managed::ManagedResource;

// Re-export topology configs with shorter paths.
pub use topology::pooled::config::Config as PoolConfig;
pub use topology::resident::config::Config as ResidentConfig;
pub use topology::service::Config as ServiceConfig;
pub use topology::transport::config::Config as TransportConfig;
pub use topology::exclusive::config::Config as ExclusiveConfig;
pub use topology::daemon::Config as DaemonConfig;
```

**Step 2: Verify topology config module paths exist**

Check each `config` module path exists in the actual source. Adjust if the structure differs (e.g., some topologies may have `Config` directly on the trait module, not in a `config` submodule).

**Step 3: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 4: Commit**

```
feat(resource): re-export runtime types and topology configs from crate root

TopologyRuntime, PoolRuntime, PoolConfig, etc. are now importable
directly from nebula_resource instead of deep module paths.
```

---

### Task 7: Wire AcquireOptions through acquire path

Currently `AcquireOptions` is defined, tested, exported — but no acquire method accepts it.

**Files:**
- Modify: `crates/resource/src/manager.rs` — add `options` param to all `acquire_*` methods
- Modify: `crates/resource/src/runtime/pool.rs` — accept and use `AcquireOptions`
- Modify: `crates/resource/src/runtime/resident.rs` — accept `AcquireOptions`
- Modify: `crates/resource/src/runtime/service.rs` — accept `AcquireOptions`
- Modify: `crates/resource/src/runtime/transport.rs` — accept `AcquireOptions`
- Modify: `crates/resource/src/runtime/exclusive.rs` — accept `AcquireOptions`
- Modify: `crates/resource/tests/basic_integration.rs` — update call sites

**Step 1: Add `options` parameter to Manager acquire methods**

For each `acquire_*` method, add `options: &AcquireOptions` parameter. Pass it through to the topology runtime. Start by defaulting behavior — just threading it through.

```rust
pub async fn acquire_pooled<R>(
    &self,
    credential: &R::Credential,
    ctx: &dyn Ctx,
    options: &AcquireOptions,  // NEW
) -> Result<ResourceHandle<R>, Error>
```

**Step 2: Add `options` to topology runtime acquire methods**

```rust
// pool.rs
pub async fn acquire(
    &self,
    resource: &R,
    resource_config: &R::Config,
    credential: &R::Credential,
    ctx: &dyn Ctx,
    release_queue: &ReleaseQueue,
    generation: u64,
    options: &AcquireOptions,  // NEW
) -> Result<ResourceHandle<R>, Error>
```

Same pattern for resident, service, transport, exclusive.

**Step 3: Use `options.deadline` in Pool acquire for semaphore timeout**

In `pool.rs acquire_semaphore_permit()`, if `options.deadline` is set, use it instead of `config.create_timeout`:

```rust
async fn acquire_semaphore_permit(&self, options: &AcquireOptions) -> Result<OwnedSemaphorePermit, Error> {
    let timeout = options
        .remaining()
        .unwrap_or(self.config.create_timeout);

    match tokio::time::timeout(timeout, self.semaphore.clone().acquire_owned()).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(_)) => Err(Error::cancelled()),
        Err(_) => Err(Error::backpressure("pool semaphore acquire timed out")),
    }
}
```

**Step 4: Update all test call sites**

Add `&AcquireOptions::default()` to all existing acquire calls in tests.

**Step 5: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 6: Commit**

```
feat(resource): wire AcquireOptions through acquire path

All acquire_* methods now accept AcquireOptions. Pool uses
deadline for semaphore timeout. Other topologies thread it
through for future use (metrics labels, priority).
```

---

### Task 8: Wire ResourceEvent emission in Manager

**Files:**
- Modify: `crates/resource/src/manager.rs` — emit events
- Modify: `crates/resource/Cargo.toml` — add nebula-eventbus dependency (if not present)

**Step 1: Check if nebula-eventbus exists and is usable**

If `nebula-eventbus` is available, use `EventBus<ResourceEvent>`. If not, use a simple `tokio::sync::broadcast` channel as a lightweight event bus within the crate.

**Step 2: Add event sender to Manager**

```rust
use tokio::sync::broadcast;

pub struct Manager {
    registry: Registry,
    recovery_groups: RecoveryGroupRegistry,
    cancel: CancellationToken,
    metrics: ResourceMetrics,
    event_tx: broadcast::Sender<ResourceEvent>,
}

impl Manager {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            // ...
            event_tx,
        }
    }

    /// Subscribe to resource lifecycle events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ResourceEvent> {
        self.event_tx.subscribe()
    }
}
```

**Step 3: Emit events in register, remove, acquire, shutdown**

```rust
// In register():
let _ = self.event_tx.send(ResourceEvent::Registered { key: key.clone() });

// In remove():
let _ = self.event_tx.send(ResourceEvent::Removed { key: key.clone() });

// In record_acquire_result():
fn record_acquire_result<R: Resource>(
    &self,
    result: &Result<ResourceHandle<R>, Error>,
    started: Instant,
) {
    match result {
        Ok(_) => {
            self.metrics.record_acquire();
            let _ = self.event_tx.send(ResourceEvent::AcquireSuccess {
                key: R::key(),
                duration: started.elapsed(),
            });
        }
        Err(e) => {
            self.metrics.record_acquire_error();
            let _ = self.event_tx.send(ResourceEvent::AcquireFailed {
                key: R::key(),
                error: e.to_string(),
            });
        }
    }
}
```

Update all `record_acquire_result` call sites to pass `Instant::now()` before the acquire call.

**Step 4: Write test for event emission**

```rust
#[tokio::test]
async fn register_emits_event() {
    let manager = Manager::new();
    let mut rx = manager.subscribe_events();
    // ... register a resource ...
    let event = rx.try_recv().unwrap();
    assert!(matches!(event, ResourceEvent::Registered { .. }));
}
```

**Step 5: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 6: Commit**

```
feat(resource): emit ResourceEvent on register/remove/acquire

Manager now broadcasts lifecycle events via tokio broadcast channel.
Subscribers can react to Registered, Removed, AcquireSuccess,
AcquireFailed events for observability and diagnostics.
```

---

### Task 9: Wire ResourceMetrics recording in release path

Currently metrics record acquire/create/destroy but NOT release. The `record_release()` method exists but is never called.

**Files:**
- Modify: `crates/resource/src/runtime/pool.rs` — record release in callback
- Modify: `crates/resource/src/runtime/service.rs` — record release for tracked tokens
- Modify: `crates/resource/src/runtime/transport.rs` — record release
- Modify: `crates/resource/src/runtime/exclusive.rs` — record release

**Step 1: Thread metrics Arc into release callbacks**

The release callbacks in `build_guarded_handle` closures need access to metrics. Options:
- Pass `Arc<ResourceMetrics>` into the closure
- Or have `ManagedResource` expose metrics ref

Since `ManagedResource` already exists and the release callbacks are built inside topology runtimes, the simplest approach is passing a cloneable metrics handle.

In `manager.rs`, pass `&self.metrics` into each acquire call so it can be threaded to release callbacks. Or better: add `metrics: Arc<ResourceMetrics>` to `ManagedResource`.

**Step 2: Update ManagedResource to hold metrics**

Check `runtime/managed.rs` structure and add metrics field if not present.

**Step 3: In pool release callback, call `metrics.record_release()`**

**Step 4: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 5: Commit**

```
feat(resource): record release metrics in pool/service/transport/exclusive

ResourceMetrics.record_release() is now called in every topology's
release callback, completing the acquire-release metrics cycle.
```

---

## Wave 2 Checkpoint

```bash
rtk cargo fmt && rtk cargo clippy -p nebula-resource -- -D warnings && rtk cargo nextest run -p nebula-resource
```

ALL PASS before proceeding.

---

## Wave 3 — API Polish

### Task 10: ScopeLevel with typed IDs

**Files:**
- Modify: `crates/resource/src/ctx.rs` — change Workflow/Execution variants
- Modify: `crates/resource/tests/basic_integration.rs` — update test constructors

**Step 1: Change ScopeLevel variants**

```rust
use nebula_core::{ExecutionId, WorkflowId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScopeLevel {
    Global,
    Organization(String),
    Project(String),
    Workflow(WorkflowId),
    Execution(ExecutionId),
}
```

**Step 2: Update all construction sites**

Grep for `ScopeLevel::Workflow(` and `ScopeLevel::Execution(` and fix.

**Step 3: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 4: Check downstream crates**

```bash
rtk cargo check --workspace
```

Fix any downstream breakage.

**Step 5: Commit**

```
refactor(resource): use WorkflowId/ExecutionId in ScopeLevel

Replaces raw String in Workflow and Execution variants with typed
IDs from nebula-core. Prevents accidental cross-variant confusion.
```

---

### Task 11: WatchdogHandle implementation

Background health probe for Service and Transport topologies.

**Files:**
- Create: `crates/resource/src/recovery/watchdog.rs`
- Modify: `crates/resource/src/recovery/mod.rs` — add module + re-exports
- Modify: `crates/resource/src/lib.rs` — re-export

**Step 1: Write WatchdogConfig**

```rust
//! Opt-in background health probe for resource runtimes.

use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Configuration for the watchdog health probe.
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// How often to run the health check.
    pub interval: Duration,
    /// Timeout for each probe attempt.
    pub probe_timeout: Duration,
    /// Number of consecutive failures before marking unhealthy.
    pub failure_threshold: u32,
    /// Number of consecutive successes to recover from unhealthy.
    pub recovery_threshold: u32,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(5),
            failure_threshold: 3,
            recovery_threshold: 1,
        }
    }
}
```

**Step 2: Write WatchdogHandle**

```rust
/// Handle to a running watchdog probe.
///
/// Drop the handle to stop the watchdog.
pub struct WatchdogHandle {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

impl WatchdogHandle {
    /// Starts a new watchdog that periodically calls `check_fn`.
    ///
    /// `on_health_change` is called when health transitions between
    /// healthy and unhealthy states.
    pub fn start<F, Fut>(
        config: WatchdogConfig,
        check_fn: F,
        on_health_change: impl Fn(bool) + Send + Sync + 'static,
        parent_cancel: CancellationToken,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), crate::Error>> + Send,
    {
        let cancel = parent_cancel.child_token();
        let token = cancel.clone();

        let join = tokio::spawn(async move {
            let mut consecutive_failures: u32 = 0;
            let mut consecutive_successes: u32 = 0;
            let mut healthy = true;

            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(config.interval) => {}
                }

                let result = tokio::time::timeout(
                    config.probe_timeout,
                    check_fn(),
                ).await;

                match result {
                    Ok(Ok(())) => {
                        consecutive_failures = 0;
                        consecutive_successes += 1;
                        if !healthy && consecutive_successes >= config.recovery_threshold {
                            healthy = true;
                            on_health_change(true);
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        consecutive_successes = 0;
                        consecutive_failures += 1;
                        if healthy && consecutive_failures >= config.failure_threshold {
                            healthy = false;
                            on_health_change(false);
                        }
                    }
                }
            }
        });

        Self { cancel, join }
    }

    /// Stops the watchdog and waits for it to finish.
    pub async fn stop(self) {
        self.cancel.cancel();
        let _ = self.join.await;
    }
}

impl Drop for WatchdogHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
```

**Step 3: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn watchdog_detects_failure() {
        let call_count = Arc::new(AtomicU32::new(0));
        let health_changed = Arc::new(AtomicBool::new(true));

        let cc = call_count.clone();
        let hc = health_changed.clone();

        let config = WatchdogConfig {
            interval: Duration::from_millis(50),
            probe_timeout: Duration::from_millis(100),
            failure_threshold: 2,
            recovery_threshold: 1,
        };

        let cancel = CancellationToken::new();
        let handle = WatchdogHandle::start(
            config,
            move || {
                let n = cc.fetch_add(1, Ordering::Relaxed);
                async move {
                    if n >= 2 {
                        Err(crate::Error::transient("probe failed"))
                    } else {
                        Ok(())
                    }
                }
            },
            move |healthy| { hc.store(healthy, Ordering::Relaxed); },
            cancel,
        );

        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!health_changed.load(Ordering::Relaxed));

        handle.stop().await;
    }
}
```

**Step 4: Add module to recovery/mod.rs**

```rust
pub mod watchdog;
pub use watchdog::{WatchdogConfig, WatchdogHandle};
```

**Step 5: Re-export from lib.rs**

```rust
pub use recovery::{WatchdogConfig, WatchdogHandle, ...};
```

**Step 6: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 7: Commit**

```
feat(resource): implement WatchdogHandle for background health probes

Periodic async health check with failure/recovery thresholds.
Integrates with CancellationToken for graceful shutdown.
For Service and Transport topologies that lack natural liveness.
```

---

## Final Checkpoint

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace
```

ALL PASS. Update context file:

```bash
# Update .claude/crates/resource.md with:
# - WatchdogHandle added
# - TopologyTag enum replaces string tags
# - AcquireOptions wired through acquire
# - ResourceEvent emitted by Manager
# - ReleaseQueue unbounded fallback
# - ResidentRuntime race fixed
# - #[non_exhaustive] on all public enums
```

---

## Summary

| Task | Wave | Type | Risk |
|------|------|------|------|
| 1. ReleaseQueue unbounded fallback | 1 | Bug fix | Low |
| 2. ResidentRuntime race fix | 1 | Bug fix | Low |
| 3. Manager::remove TOCTOU | 1 | Bug fix | Low |
| 4. `#[non_exhaustive]` on enums | 2 | API safety | Medium |
| 5. TopologyTag enum | 2 | Refactor | Low |
| 6. Re-export runtime types | 2 | DX | Low |
| 7. Wire AcquireOptions | 2 | Feature | Medium |
| 8. Wire ResourceEvent emission | 2 | Feature | Low |
| 9. Wire release metrics | 2 | Feature | Low |
| 10. ScopeLevel typed IDs | 3 | Refactor | Medium |
| 11. WatchdogHandle | 3 | Feature | Low |
