# nebula-resource — Pooling

[`PoolRuntime<R>`] is the runtime side of the `Pooled` topology. It manages a
bounded set of `R::Runtime` instances with semaphore-controlled access,
idle/lifetime expiry, FIFO/LIFO checkout ordering, configurable warmup, and
optional per-checkout health probes. The corresponding trait is
[`Pooled`](crate::topology::pooled::Pooled), and the configuration object is
[`PoolConfig`].

---

## Table of Contents

- [How the Pool Works](#how-the-pool-works)
- [PoolConfig Reference](#poolconfig-reference)
- [Idle Selection Strategy](#idle-selection-strategy)
- [Warmup Strategy](#warmup-strategy)
- [Test On Checkout](#test-on-checkout)
- [PoolStats](#poolstats)
- [Lifecycle of an Instance](#lifecycle-of-an-instance)
- [Configuration Examples](#configuration-examples)

---

## How the Pool Works

```text
PoolRuntime<R>
  ├── idle_queue: VecDeque<IdleEntry<R::Runtime>>   // ordered by checkout strategy
  ├── semaphore:  Semaphore (max_size permits)      // bounds in-flight + idle
  ├── active:     AtomicUsize
  └── waiters:    AtomicUsize
```

**Acquire** (`Manager::acquire_pooled` / `acquire_pooled_default`):

1. Atomically increment waiter count (RAII counter).
2. Acquire one semaphore permit (waits up to the resilience-supplied timeout).
3. Pop from `idle_queue` (`Lifo`: back / `Fifo`: front, per
   `PoolConfig::strategy`).
4. If `test_on_checkout` is true, call `Resource::check`. Discard on `Err`.
5. If no idle instance (or all discarded), call `Resource::create` —
   bounded by `max_concurrent_creates` to prevent thundering on the backend.
6. Wrap the runtime in a [`ResourceGuard`] with a release callback.
7. Emit `ResourceEvent::AcquireSuccess { duration }`.

**Release** (`ResourceGuard` drop):

- If `taint()` was called: skip recycle, call `Resource::destroy` via the
  `ReleaseQueue`, emit `ResourceEvent::Released { tainted: true, .. }`.
- Otherwise: call `Pooled::recycle` synchronously to obtain a
  `RecycleDecision`:
  - `Keep` → `Pooled::is_broken` runs; if `Healthy`, push back to
    `idle_queue` and release the semaphore permit.
  - `Drop` → `Resource::destroy` via the `ReleaseQueue`; semaphore permit
    released.
- Emit `ResourceEvent::Released { held, tainted }`.

**Background maintenance** (`maintenance_interval`):

- Sweeps the idle queue. Instances older than `max_lifetime` or idle longer
  than `idle_timeout` are evicted via `Resource::destroy`.

---

## PoolConfig Reference

```rust,ignore
use std::time::Duration;
use nebula_resource::PoolConfig;
use nebula_resource::topology::pooled::{PoolStrategy, WarmupStrategy};
```

The struct (`crate::topology::pooled::config::Config`, re-exported as
`nebula_resource::PoolConfig`):

```rust,ignore
pub struct PoolConfig {
    pub min_size:               u32,
    pub max_size:               u32,
    pub idle_timeout:           Option<Duration>,
    pub max_lifetime:           Option<Duration>,
    pub create_timeout:         Duration,
    pub strategy:               PoolStrategy,
    pub warmup:                 WarmupStrategy,
    pub test_on_checkout:       bool,
    pub maintenance_interval:   Duration,
    pub max_concurrent_creates: u32,
}
```

| Field | Default | Effect |
|-------|---------|--------|
| `min_size` | `1` | Pool proactively maintains at least this many idle instances (driven by `WarmupStrategy` at startup). |
| `max_size` | `10` | Hard cap on instances (idle + checked out). Acquires beyond this wait on the semaphore. |
| `idle_timeout` | `Some(5 min)` | Evicts instances idle longer than this. `None` disables idle eviction. |
| `max_lifetime` | `Some(30 min)` | Evicts instances older than this regardless of idle time. `None` disables. |
| `create_timeout` | `30 s` | Per-call timeout for `Resource::create`. |
| `strategy` | `Lifo` | `Lifo` (hot working set) or `Fifo` (even spread). See [Idle Selection Strategy](#idle-selection-strategy). |
| `warmup` | `WarmupStrategy::None` | Pre-create instances at pool startup. See [Warmup Strategy](#warmup-strategy). |
| `test_on_checkout` | `false` | If `true`, runs `Resource::check` on each checkout; `Err` discards the instance and creates fresh. |
| `maintenance_interval` | `30 s` | Background sweep interval for idle/lifetime eviction. |
| `max_concurrent_creates` | `3` | Caps concurrent `Resource::create` calls during cold-start / warmup. |

`PoolConfig::default()` produces a sensible starting point; tune `min_size`/
`max_size`/`max_lifetime` against your backend's connection-budget and TTL.

`PoolConfig::validate()` enforces `min_size <= max_size` and non-zero
`max_size`. `Manager::register_pooled` / `register_pooled_with` call this
implicitly.

---

## Idle Selection Strategy

```rust,ignore
pub enum PoolStrategy {
    Lifo,   // pop_back  — return most recently used instance (default)
    Fifo,   // pop_front — return oldest idle instance
}
```

### When to use LIFO (default)

- Keeps a small "hot" working set active; lets excess idle instances expire
  naturally via `idle_timeout`.
- Ideal when `min_size` is much smaller than `max_size` and load is bursty:
  most requests hit the same few connections; the rest idle out.
- PostgreSQL connection pools behind PgBouncer often benefit from LIFO.

### When to use FIFO

- Equal distribution across all pooled instances.
- Prevents any single connection from going stale under steady load.
- Use when all instances have equivalent cost AND the pool is consistently
  near-full utilisation.

---

## Warmup Strategy

`WarmupStrategy` controls how `min_size` instances are created at pool
startup:

```rust,ignore
pub enum WarmupStrategy {
    None,                                    // create on demand (default)
    Sequential,                              // one at a time
    Parallel,                                // all at once
    Staggered { interval: Duration },        // with delay between creations
}
```

- `None` — first acquire pays the cold-start cost. Cheapest, slowest first
  request.
- `Sequential` — `min_size` instances created back-to-back during
  `Manager::register_pooled*`. Predictable startup latency.
- `Parallel` — fastest warmup but spikes connection count; verify the
  backend tolerates it.
- `Staggered { interval }` — connection rate-limited startup. Use when the
  backend has connection-rate caps (e.g., shared cloud DB).

`max_concurrent_creates` clamps `Parallel` and the active count for
`Staggered` to avoid overwhelming the backend.

---

## Test On Checkout

`test_on_checkout: true` runs `Resource::check` on every checkout. If
`check` returns `Err`, the instance is discarded and a fresh one is
created:

```rust,ignore
PoolConfig { test_on_checkout: true, ..PoolConfig::default() }
```

**Cost:** one round-trip per acquire. **Benefit:** never hand out a dead
connection. Use when:

- Your driver's `Resource::is_broken` cannot detect TCP-half-open or
  server-side timeouts cheaply.
- The cost of a failed query (caller-side retry, transaction abort) is
  much higher than a connection-test ping.

For most adapters, prefer letting `Pooled::is_broken` (synchronous, runs
in `Drop`) handle obvious closures and skip `test_on_checkout`.

---

## PoolStats

`Manager::lookup::<R>(scope)?.topology()` returns the `TopologyRuntime<R>`
variant; `PoolRuntime<R>::stats()` exposes runtime counters:

```rust,ignore
pub struct PoolStats {
    pub active:                 usize,    // currently checked out
    pub idle:                   usize,    // in idle_queue
    pub waiters:                usize,    // blocked on the semaphore
    pub total_acquisitions:     u64,
    pub total_creations:        u64,
    pub total_destroys:         u64,
}
```

For aggregate cross-pool counters, use `Manager::metrics()` — see
[`api-reference.md`](api-reference.md) for `ResourceOpsMetrics` /
`ResourceOpsSnapshot`.

---

## Lifecycle of an Instance

```text
Resource::create(config, scheme, ctx)
  │
  ▼ [instance enters pool, idle_queue.push_back]
  │
  ├─ idle in VecDeque
  │    │ idle_timeout exceeded     → Resource::destroy (background sweep)
  │    │ max_lifetime exceeded     → Resource::destroy (background sweep)
  │    └─ test_on_checkout enabled → Resource::check on each checkout
  │         └─ Err                 → Resource::destroy + create fresh
  │
  ├─ checked out (ResourceGuard held by caller)
  │    │ guard.taint() called      → Resource::destroy via ReleaseQueue
  │    └─ guard dropped            → Pooled::recycle:
  │         ├─ RecycleDecision::Keep + is_broken=Healthy → push back to idle
  │         └─ RecycleDecision::Drop OR is_broken=Broken → Resource::destroy
  │
  └─ Manager::graceful_shutdown    → drain in-flight + Resource::destroy on idle
```

`Resource::destroy` runs through `ReleaseQueue` (per-Manager background
worker pool) so caller-side drop is non-blocking. See `recovery.md` for
the `RecoveryGate` interaction when `is_broken` returns `Broken` repeatedly.

---

## Configuration Examples

### Minimal — defaults

```rust,ignore
use nebula_resource::PoolConfig;

let config = PoolConfig::default();
// min_size=1, max_size=10, idle=5min, lifetime=30min, Lifo, no warmup,
// no test_on_checkout, maintenance every 30s, max_concurrent_creates=3
```

### High-throughput API backend

```rust,ignore
use std::time::Duration;
use nebula_resource::PoolConfig;
use nebula_resource::topology::pooled::{PoolStrategy, WarmupStrategy};

PoolConfig {
    min_size: 5,
    max_size: 50,
    idle_timeout: Some(Duration::from_secs(300)),
    max_lifetime: Some(Duration::from_secs(3600)),
    create_timeout: Duration::from_secs(5),
    strategy: PoolStrategy::Lifo,
    warmup: WarmupStrategy::Parallel,
    max_concurrent_creates: 10,
    ..PoolConfig::default()
}
```

### Single-tenant background worker

```rust,ignore
PoolConfig {
    min_size: 1,
    max_size: 3,
    idle_timeout: Some(Duration::from_secs(600)),
    strategy: PoolStrategy::Fifo,
    ..PoolConfig::default()
}
```

### Cloud backend with connection-rate caps

```rust,ignore
PoolConfig {
    min_size: 5,
    max_size: 25,
    warmup: WarmupStrategy::Staggered {
        interval: Duration::from_millis(500),
    },
    max_concurrent_creates: 1,    // serial cold-start to respect rate limits
    ..PoolConfig::default()
}
```

### Strict health verification (test on each checkout)

```rust,ignore
PoolConfig {
    test_on_checkout: true,
    ..PoolConfig::default()
}
```

---

## Integration with Resilience

Per-acquire timeout, retry policy, and recovery-gate admission belong on
`AcquireResilience` / `RecoveryGate`, not on `PoolConfig`. Wire them in
via `RegisterOptions`:

```rust,ignore
use std::sync::Arc;
use nebula_resource::{
    AcquireResilience, RecoveryGate, RecoveryGateConfig, RegisterOptions,
};

let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register_pooled_with(
    PostgresResource,
    pg_config,
    PoolConfig::default(),
    RegisterOptions {
        resilience:    Some(AcquireResilience::standard()),
        recovery_gate: Some(gate),
        ..RegisterOptions::default()
    },
)?;
```

See [`api-reference.md`](api-reference.md) for `RegisterOptions` field
semantics and [`recovery.md`](recovery.md) for `RecoveryGate` behavior.
