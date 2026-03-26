# Pooling

Pool topology — bounded, recycling pool of interchangeable resource instances.

---

## Overview

The pool topology manages N instances of a resource behind a semaphore.
Callers check out an instance, use it, and return it on handle drop.
Instances are recycled, health-checked, and evicted based on configurable policies.

---

## Pooled Trait

Implement [`Pooled`] (extends [`Resource`]) to add pool-aware lifecycle hooks:

```rust,ignore
impl Pooled for MyResource {
    // Sync O(1) check — runs in Drop path. NO async, NO I/O.
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck {
        if runtime.is_closed() {
            BrokenCheck::Broken("connection closed".into())
        } else {
            BrokenCheck::Healthy
        }
    }

    // Async recycle — runs after handle drop, in ReleaseQueue worker.
    async fn recycle(
        &self, runtime: &Self::Runtime, metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, Self::Error> {
        if metrics.age() > Duration::from_secs(3600) {
            Ok(RecycleDecision::Drop)
        } else {
            Ok(RecycleDecision::Keep)
        }
    }

    // Per-checkout setup — runs after idle pop, before caller gets handle.
    async fn prepare(
        &self, runtime: &Self::Runtime, ctx: &dyn Ctx,
    ) -> Result<(), Self::Error> {
        // e.g., SET search_path for tenant isolation
        Ok(())
    }
}
```

All three methods have sensible defaults (healthy, keep, no-op).

---

## Pool Configuration

```rust,ignore
use nebula_resource::PoolConfig;

let config = PoolConfig {
    min_size: 2,                              // keep at least 2 idle
    max_size: 20,                             // semaphore bound
    idle_timeout: Some(Duration::from_secs(300)),  // evict after 5min idle
    max_lifetime: Some(Duration::from_secs(1800)), // evict after 30min total
    create_timeout: Duration::from_secs(30),  // per-instance create deadline
    strategy: PoolStrategy::Lifo,             // reuse most-recent (default)
    warmup: WarmupStrategy::None,             // create on demand (default)
    test_on_checkout: false,                  // skip check() on acquire
    maintenance_interval: Duration::from_secs(30),
    max_concurrent_creates: 3,
};
```

### Idle Selection Strategy

| Strategy | Behavior | Best for |
|----------|----------|----------|
| `Lifo` (default) | Reuse most recently returned | Warm caches, fewer idle instances |
| `Fifo` | Reuse oldest returned | Even load distribution |

### Warmup Strategy

| Strategy | Behavior |
|----------|----------|
| `None` (default) | Create on first acquire |
| `Sequential` | Create `min_size` instances one at a time |
| `Parallel` | Create `min_size` instances concurrently |
| `Staggered { interval }` | Create with delay between each |

---

## Acquire Flow

```text
acquire_pooled()
  |
  +-- Semaphore::acquire (respects max_size)
  |    \-- timeout -> ErrorKind::Backpressure
  |
  +-- Try idle queue (LIFO/FIFO)
  |    +-- Found -> is_broken()?
  |    |    +-- Broken -> destroy, try next
  |    |    \-- Healthy -> test_on_checkout?
  |    |         +-- check() fails -> destroy, try next
  |    |         \-- OK -> prepare(ctx) -> return handle
  |    \-- Empty -> create new
  |
  +-- Resource::create(config, credential, ctx)
  |    \-- CreateGuard wraps entry (cancel-safety)
  |
  +-- prepare(ctx)
  |
  \-- Return ResourceHandle::guarded_with_permit(...)
```

---

## Release Flow

When a `ResourceHandle` is dropped:

```text
Drop::drop()
  |
  +-- Extract semaphore permit (returned even on panic)
  |
  +-- catch_unwind { on_release(lease, tainted) }
  |    |
  |    \-- ReleaseQueue::submit(async {
  |         +-- tainted? -> destroy
  |         +-- stale fingerprint? -> destroy
  |         +-- max_lifetime exceeded? -> destroy
  |         +-- recycle() -> Drop? -> destroy
  |         \-- Keep -> push to idle queue
  |       })
  |
  \-- _permit_guard drops -> semaphore slot returned
```

**Key invariant:** The semaphore permit is returned *before* the async recycle
runs. This means a new caller can acquire during the recycle check. This is
intentional — it prevents recycle from becoming a bottleneck.

---

## Fingerprint-Based Eviction

Implement `ResourceConfig::fingerprint()` to enable config-change detection:

```rust,ignore
impl ResourceConfig for MyConfig {
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.host.hash(&mut hasher);
        self.port.hash(&mut hasher);
        hasher.finish()
    }
}
```

When `Manager::reload_config()` is called:
1. New config is validated and swapped via `ArcSwap`
2. Generation counter increments
3. Pool fingerprint updates to `new_config.fingerprint()`
4. Idle instances with old fingerprint are evicted on next maintenance sweep

---

## Maintenance Sweep

A background maintenance sweep runs every `maintenance_interval` (default 30s).
It evicts idle instances that match any of:
- **Idle timeout exceeded** — sitting idle longer than `idle_timeout`
- **Max lifetime exceeded** — total age exceeds `max_lifetime`
- **Stale fingerprint** — config changed since instance was created

Evicted instances are destroyed via `Resource::destroy()` through the `ReleaseQueue`.

---

## Cancel-Safety

The pool uses `CreateGuard` to handle cancel-safety during the
create -> prepare -> handle-construction sequence. If the future is cancelled
(e.g., by `tokio::select!` or timeout) after `create()` succeeds but before
the handle is built, the guard logs a warning and drops the runtime —
triggering its native `Drop` impl to close sockets/files.
