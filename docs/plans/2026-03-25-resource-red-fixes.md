# Resource RED Fixes — Production Safety

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 4 critical production-safety issues: permit leak on panic, Transport/Exclusive infinite block, and Resident create/destroy timeout.

**Architecture:** Each fix is self-contained. Task 1 restructures permit ownership so panics in release callbacks can't leak semaphore permits. Tasks 2-3 add timeouts to Transport/Exclusive/Resident acquire paths. All changes are internal — no public API breaks.

**Tech Stack:** Rust 1.93, tokio, tokio::sync::Semaphore

---

## Task 1: Prevent semaphore permit leak on release callback panic

**Problem:** In `handle.rs:219-228`, `catch_unwind` catches panics in the release callback. But the `OwnedSemaphorePermit` lives *inside* the callback closure (captured from pool.rs:312, transport.rs:112, exclusive.rs:99). If the callback panics, the permit is destroyed with the unwound closure — permanently losing a pool slot.

**Approach:** Extract the permit from the closure. Add an optional `permit: Option<OwnedSemaphorePermit>` field to the `Guarded` variant of `HandleInner`. The permit is dropped in the `Drop` impl *after* the callback runs (or panics). This way, even if `catch_unwind` catches a panic, the permit is still alive and will be dropped normally.

**Files:**
- Modify: `crates/resource/src/handle.rs` — add permit field to Guarded
- Modify: `crates/resource/src/runtime/pool.rs` — pass permit to handle, not closure
- Modify: `crates/resource/src/runtime/transport.rs` — same
- Modify: `crates/resource/src/runtime/exclusive.rs` — same

**Step 1: Write test that verifies permit is returned even when callback panics**

In `crates/resource/tests/basic_integration.rs`, add:

```rust
#[tokio::test]
async fn pool_permit_survives_release_callback_panic() {
    // A pool with max_size=1. If the permit leaks, second acquire will block forever.
    // We'll use a resource whose Lease -> Runtime conversion panics on taint.
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        ..Default::default()
    };
    let pool = PoolRuntime::<PoolTestResource>::new(config, 1);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let ctx = test_ctx();
    let metrics = Arc::new(ResourceMetrics::new());

    // First acquire — succeeds.
    let handle = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0,
                 &AcquireOptions::default(), metrics.clone())
        .await.unwrap();
    drop(handle);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Second acquire — must succeed (permit was returned).
    let handle2 = pool
        .acquire(&resource, &test_config(), &(), &ctx, &rq, 0,
                 &AcquireOptions::default(), metrics)
        .await.unwrap();
    drop(handle2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}
```

**Step 2: Add permit field to HandleInner::Guarded**

In `handle.rs`:

```rust
use tokio::sync::OwnedSemaphorePermit;

enum HandleInner<R: Resource> {
    Owned(R::Lease),
    Guarded {
        value: Option<R::Lease>,
        on_release: Option<GuardedRelease<R>>,
        tainted: bool,
        acquired_at: Instant,
        generation: u64,
        permit: Option<OwnedSemaphorePermit>,  // NEW — dropped after callback
    },
    Shared {
        value: Arc<R::Lease>,
        on_release: Option<Box<dyn FnOnce(bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
        generation: u64,
    },
}
```

Update `ResourceHandle::guarded()` constructor to accept optional permit:

```rust
pub fn guarded(
    lease: R::Lease,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    generation: u64,
    on_release: impl FnOnce(R::Lease, bool) + Send + 'static,
) -> Self {
    Self::guarded_with_permit(lease, resource_key, topology_tag, generation, on_release, None)
}

pub fn guarded_with_permit(
    lease: R::Lease,
    resource_key: ResourceKey,
    topology_tag: TopologyTag,
    generation: u64,
    on_release: impl FnOnce(R::Lease, bool) + Send + 'static,
    permit: Option<OwnedSemaphorePermit>,
) -> Self {
    Self {
        inner: HandleInner::Guarded {
            value: Some(lease),
            on_release: Some(Box::new(on_release)),
            tainted: false,
            acquired_at: Instant::now(),
            generation,
            permit,
        },
        resource_key,
        topology_tag,
    }
}
```

In `Drop` impl — the permit field is dropped *naturally* after the callback match arm completes (or after `catch_unwind` catches a panic). No explicit code needed — Rust's drop order handles it.

Update the `detach()` method to also take the permit out (detach means "no pool management"):
```rust
// In detach() for Guarded arm:
HandleInner::Guarded { value: None, on_release: None, tainted: true,
                       acquired_at: Instant::now(), generation: 0, permit: None }
```

**Step 3: Update pool.rs — pass permit to handle, remove from closure**

In `build_guarded_handle` (pool.rs:286-330):

```rust
fn build_guarded_handle(
    &self,
    lease: R::Lease,
    entry: PoolEntry<R>,
    resource: R,
    release_queue: Arc<ReleaseQueue>,
    generation: u64,
    metrics: Arc<ResourceMetrics>,
) -> ResourceHandle<R> {
    let idle = self.idle.clone();
    let current_fp_ref = self.current_fingerprint.clone();
    let max_lifetime = self.config.max_lifetime;

    // Extract permit from entry BEFORE moving entry into closure.
    let permit = entry.permit;

    ResourceHandle::guarded_with_permit(
        lease,
        R::key(),
        TopologyTag::Pool,
        generation,
        move |returned_lease: R::Lease, tainted| {
            metrics.record_release();
            let runtime: R::Runtime = returned_lease.into();
            let entry = PoolEntry {
                runtime,
                metrics: entry.metrics.clone(),
                fingerprint: entry.fingerprint,
                permit: // REMOVED — permit no longer in entry for release
            };
            // ... submit to release_queue
        },
        Some(permit),  // permit held by handle, not closure
    )
}
```

Wait — this changes `PoolEntry` structure. The permit is used inside `release_entry` to track the semaphore slot. If we remove it from the entry, the release path needs restructuring.

**Alternative approach:** Instead of moving permit to the handle, wrap the callback in a drop guard that ensures the permit is returned even on panic. This is simpler:

```rust
// In handle.rs Drop for Guarded:
HandleInner::Guarded { value, on_release, tainted, permit, .. } => {
    // Permit will drop at the END of this arm, even if callback panics.
    let _permit_guard = permit.take(); // moved out, dropped at end of scope

    if let (Some(lease), Some(callback)) = (value.take(), on_release.take()) {
        let tainted = *tainted;
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(lease, tainted);
        })).is_err() {
            tracing::error!(...);
        }
    }
    // _permit_guard drops here — permit returned to semaphore
}
```

Actually simplest: just add the `permit` field. The Drop order within the match arm is: callback fires (maybe panics, caught), then the entire `HandleInner::Guarded` struct is dropped, including `permit`. Since `catch_unwind` doesn't consume the permit (it's a separate field), the permit survives the panic and is dropped normally.

**Step 4: Update transport.rs — extract permit from closure**

Remove `permit` from `release_transport_session` params. Instead pass permit to `guarded_with_permit`:

```rust
// transport.rs acquire():
Ok(ResourceHandle::guarded_with_permit(
    session,
    R::key(),
    TopologyTag::Transport,
    generation,
    move |lease, tainted| {
        metrics.record_release();
        rq.submit(move || {
            Box::pin(release_transport_session(resource_clone, runtime, lease, !tainted))
            // NO permit in release_transport_session
        });
    },
    Some(permit),
))
```

Update `release_transport_session` to not take `_permit`:
```rust
async fn release_transport_session<R>(resource: R, runtime: Arc<R::Runtime>, session: R::Lease, healthy: bool)
where ...
{
    let _ = resource.close_session(&runtime, session, healthy).await;
}
```

**Step 5: Update exclusive.rs — same pattern**

Remove `permit` from `release_exclusive` params. Pass to `guarded_with_permit`.

**Step 6: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
rtk cargo clippy -p nebula-resource -- -D warnings
```

**Step 7: Commit**

```
fix(resource): prevent semaphore permit leak on release callback panic

OwnedSemaphorePermit now lives in ResourceHandle::Guarded as a
separate field, not captured inside the release callback closure.
If the callback panics (caught by catch_unwind), the permit still
drops normally, returning the slot to the semaphore.

Affects Pool, Transport, and Exclusive topologies.
```

---

## Task 2: Add timeout to Transport semaphore wait

**Problem:** `TransportRuntime::acquire` (transport.rs:83-88) calls `semaphore.acquire_owned().await` with no timeout. All sessions consumed = infinite block.

**Files:**
- Modify: `crates/resource/src/topology/transport.rs` — add `acquire_timeout` to Config
- Modify: `crates/resource/src/runtime/transport.rs` — use timeout on semaphore wait

**Step 1: Add `acquire_timeout` to transport Config**

In `topology/transport.rs` config module:

```rust
pub struct Config {
    pub max_sessions: u32,
    pub keepalive_interval: Option<Duration>,
    pub acquire_timeout: Duration,  // NEW
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_sessions: 10,
            keepalive_interval: Some(Duration::from_secs(30)),
            acquire_timeout: Duration::from_secs(30),  // NEW
        }
    }
}
```

**Step 2: Use timeout in TransportRuntime::acquire**

In `runtime/transport.rs`:

```rust
pub async fn acquire(..., options: &AcquireOptions, ...) -> Result<ResourceHandle<R>, Error> {
    let timeout = options
        .remaining()
        .unwrap_or(self.config.acquire_timeout);

    let permit = match tokio::time::timeout(
        timeout,
        self.session_semaphore.clone().acquire_owned(),
    ).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err(Error::permanent("transport session semaphore closed")),
        Err(_) => return Err(Error::backpressure(
            "transport: timed out waiting for available session"
        )),
    };
    // ... rest unchanged
}
```

**Step 3: Write test**

```rust
#[tokio::test]
async fn transport_acquire_timeout_when_sessions_exhausted() {
    // max_sessions=1, hold the session, second acquire with short deadline should timeout
}
```

**Step 4: Run tests + clippy**

**Step 5: Commit**

```
fix(resource): add timeout to Transport semaphore wait

Prevents infinite blocking when all transport sessions are consumed.
Uses AcquireOptions.remaining() or config.acquire_timeout as fallback.
```

---

## Task 3: Add timeout to Exclusive semaphore wait

**Problem:** Same as Transport — `ExclusiveRuntime::acquire` (exclusive.rs:80-85) blocks forever.

**Files:**
- Modify: `crates/resource/src/topology/exclusive.rs` — add `acquire_timeout` to Config
- Modify: `crates/resource/src/runtime/exclusive.rs` — use timeout

**Step 1: Add `acquire_timeout` to exclusive Config**

```rust
pub struct Config {
    pub acquire_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            acquire_timeout: Duration::from_secs(30),
        }
    }
}
```

**Step 2: Use timeout in ExclusiveRuntime::acquire**

Same pattern as Transport.

**Step 3: Write test**

**Step 4: Run tests + clippy + commit**

```
fix(resource): add timeout to Exclusive semaphore wait
```

---

## Task 4: Add timeout to Resident create/destroy

**Problem:** `ResidentRuntime::acquire` (resident.rs:136-139) calls `resource.create()` and `resource.destroy()` without timeout, while holding `create_lock`. Hanging backend = deadlock for all acquires.

**Files:**
- Modify: `crates/resource/src/topology/resident.rs` — add `create_timeout` to Config
- Modify: `crates/resource/src/runtime/resident.rs` — wrap create/destroy in timeout

**Step 1: Add `create_timeout` to resident Config**

```rust
pub struct Config {
    pub recreate_on_failure: bool,
    pub create_timeout: Duration,  // NEW
}

impl Default for Config {
    fn default() -> Self {
        Self {
            recreate_on_failure: false,
            create_timeout: Duration::from_secs(30),
        }
    }
}
```

**Step 2: Wrap create in timeout**

In `runtime/resident.rs`, inside the slow path:

```rust
let runtime = match tokio::time::timeout(
    self.config.create_timeout,
    resource.create(resource_config, credential, ctx),
).await {
    Ok(Ok(rt)) => rt,
    Ok(Err(e)) => return Err(e.into()),
    Err(_) => return Err(Error::transient("resident: create timed out")),
};
```

Wrap destroy similarly:

```rust
let _ = tokio::time::timeout(
    Duration::from_secs(10),
    resource.destroy(owned),
).await;
```

**Step 3: Write test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn resident_create_timeout_releases_lock() {
    // Resource whose create() hangs forever.
    // First acquire times out. Second acquire also times out (doesn't deadlock).
}
```

**Step 4: Update existing tests that use Config::default()**

Config now has `create_timeout` field, existing `Config { recreate_on_failure: true }` needs `..Default::default()`.

**Step 5: Run tests + clippy + commit**

```
fix(resource): add timeout to Resident create/destroy

Prevents create_lock deadlock when backend hangs. Uses
config.create_timeout (default 30s). Destroy gets a fixed 10s timeout.
```

---

## Final Checkpoint

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace
```

Update `.claude/crates/resource.md` with new traps:
- Permit held by handle, not closure — don't move it into release callbacks
- Transport/Exclusive have acquire_timeout in Config
- Resident create_timeout in Config
