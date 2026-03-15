# nebula-resource — Pooling

`Pool<R>` is the core concurrency primitive in `nebula-resource`.
It manages a bounded set of `R::Instance` values with semaphore-controlled
access, idle-expiry, FIFO/LIFO selection, circuit-breaker-guarded creation,
and optional auto-scaling.

---

## Table of Contents

- [How the Pool Works](#how-the-pool-works)
- [PoolConfig Reference](#poolconfig-reference)
- [Idle Selection Strategy](#idle-selection-strategy)
- [Backpressure Policies](#backpressure-policies)
- [Circuit Breakers](#circuit-breakers)
- [Pool Stats and Latency Histograms](#pool-stats-and-latency-histograms)
- [Auto-Scaling](#auto-scaling)
- [Lifecycle of an Instance](#lifecycle-of-an-instance)
- [Configuration Examples](#configuration-examples)

---

## How the Pool Works

```
Pool<R>
  ├── idle_queue: VecDeque<IdleEntry<R::Instance>>
  ├── semaphore:  Semaphore (max_size permits)
  ├── active:     AtomicUsize
  ├── waiters:    AtomicUsize
  ├── create_breaker: Option<CircuitBreaker>
  └── recycle_breaker: Option<CircuitBreaker>
```

**Acquire:**
1. Atomically increment waiter count (RAII `CounterGuard`).
2. Acquire one semaphore permit per the configured `PoolBackpressurePolicy`.
3. Pop from `idle_queue` (Fifo: front, Lifo: back).
4. If an idle instance is found, call `Resource::is_reusable`. Discard if false.
5. If no idle instance (or all discarded), call `Resource::create`.
6. Wrap the instance in a `Guard<T>` with a release callback.
7. Record latency in the HDR histogram.
8. Emit `ResourceEvent::Acquired { wait_duration }`.

**Release (Guard drop):**
- If tainted: call `Resource::cleanup`, emit `CleanedUp { reason: Tainted }`.
- Otherwise: call `Resource::recycle`, push back to `idle_queue`,
  release semaphore permit, emit `ResourceEvent::Released { usage_duration }`.

**Idle expiry** (background maintenance or lazy on acquire):
- Instances older than `max_lifetime` or idle longer than `idle_timeout`
  are evicted by calling `Resource::cleanup`.

---

## PoolConfig Reference

```rust
pub struct PoolConfig {
    pub min_size: usize,                          // default: 1
    pub max_size: usize,                          // default: 10
    pub acquire_timeout: Duration,                // default: 30s
    pub idle_timeout: Duration,                   // default: 10 min
    pub max_lifetime: Duration,                   // default: 30 min
    pub validation_interval: Duration,            // default: 60s
    pub maintenance_interval: Option<Duration>,   // default: None
    pub strategy: PoolStrategy,                   // default: Fifo
    pub backpressure_policy: Option<PoolBackpressurePolicy>,
    pub create_breaker: Option<CircuitBreakerConfig>,
    pub recycle_breaker: Option<CircuitBreakerConfig>,
    pub create_timeout: Option<Duration>,
    pub recycle_timeout: Option<Duration>,
}
```

| Field | Effect |
|-------|--------|
| `min_size` | Pool proactively creates instances to maintain at least this many idle connections. |
| `max_size` | Hard cap on semaphore permits. Callers block/fail when all permits are taken. |
| `acquire_timeout` | Upper bound for waiting for a permit (used by `BoundedWait` default policy). |
| `idle_timeout` | Evicts instances idle longer than this. Prevents stale connections. |
| `max_lifetime` | Evicts instances regardless of idle time. Forces periodic reconnect. |
| `validation_interval` | How often `Resource::is_reusable` is called on idle instances. |
| `maintenance_interval` | If set, spawns a background task that runs idle-expiry on this interval. |
| `strategy` | `Fifo` (even distribution) or `Lifo` (hot working set). |
| `backpressure_policy` | Behaviour when no permit is available. See [Backpressure Policies](#backpressure-policies). |
| `create_breaker` | Circuit breaker for `Resource::create`. Opens after N consecutive failures. |
| `recycle_breaker` | Circuit breaker for `Resource::recycle`. |
| `create_timeout` | Per-call timeout for `Resource::create`. Defaults to `acquire_timeout`. |
| `recycle_timeout` | Per-call timeout for `Resource::recycle`. |

**Validation:** `PoolConfig::validate()` enforces `min_size <= max_size` and
non-zero `max_size`. Call it explicitly or let `Manager::register` call it for you.

---

## Idle Selection Strategy

```rust
pub enum PoolStrategy {
    Fifo,   // pop_front — return oldest idle instance
    Lifo,   // pop_back  — return most recently used instance
}
```

### When to use FIFO (default)

- Equal distribution across all pooled instances.
- Prevents any single connection from going stale.
- Recommended when all instances have equal cost and the pool is fully utilised.

### When to use LIFO

- Keeps a small "hot" working set active; lets excess idle instances expire
  naturally via `idle_timeout`.
- Ideal when `min_size` is much smaller than `max_size` and load is bursty:
  most requests hit the same few connections; the rest idle out.
- Database connection pools behind a PgBouncer often benefit from LIFO.

---

## Backpressure Policies

Configure how the pool behaves when all `max_size` permits are taken.

### `FailFast`

```rust
PoolBackpressurePolicy::FailFast
```

Immediately returns `Error::PoolExhausted`. Use for latency-sensitive paths
where waiting would be worse than failing and retrying at a higher level.

### `BoundedWait`

```rust
PoolBackpressurePolicy::BoundedWait { timeout: Duration::from_secs(5) }
```

Waits up to `timeout` for a permit. Returns `Error::PoolExhausted` if none
becomes available. This is the default when `backpressure_policy` is `None`
(uses `acquire_timeout`).

### `Adaptive`

```rust
PoolBackpressurePolicy::Adaptive(AdaptiveBackpressurePolicy {
    high_pressure_utilization: 0.8,   // 80% active/max
    high_pressure_waiters: 8,
    low_pressure_timeout: Duration::from_secs(30),
    high_pressure_timeout: Duration::from_millis(100),
})
```

At each acquire, the pool computes `utilisation = active / max_size` and
checks waiter count. If either threshold is exceeded, it uses
`high_pressure_timeout`; otherwise `low_pressure_timeout`.

**Typical tuning:**
- Set `high_pressure_timeout` to your SLA budget minus downstream latency.
- Set `low_pressure_timeout` to match `acquire_timeout`.
- Lower `high_pressure_utilization` to start shedding load earlier.

---

## Circuit Breakers

Circuit breakers protect against cascading failures when `Resource::create` or
`Resource::recycle` fails repeatedly.

```rust
// Enable standard circuit breakers for both operations:
let config = PoolConfig::default().with_standard_breakers();

// Or configure per-operation:
use nebula_resilience::CircuitBreakerConfig;
let config = PoolConfig {
    create_breaker: Some(CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        timeout: Duration::from_secs(30),
    }),
    recycle_breaker: Some(CircuitBreakerConfig {
        failure_threshold: 3,
        success_threshold: 1,
        timeout: Duration::from_secs(10),
    }),
    ..PoolConfig::default()
};
```

When a circuit breaker opens, `Pool::acquire` returns
`Error::CircuitBreakerOpen { retry_after }` immediately without attempting
the operation. The `EventBus` emits `ResourceEvent::CircuitBreakerOpen`.

Circuit breakers for `create` and `recycle` are **independent** — a failing
recycle does not prevent new creates.

---

## Pool Stats and Latency Histograms

```rust
let stats: PoolStats = manager.pool::<MyResource>().unwrap().stats()?;

println!("active: {}, idle: {}", stats.active, stats.idle);
println!("total acquired: {}", stats.total_acquisitions);
println!("times exhausted: {}", stats.exhausted_count);

if let Some(lat) = stats.acquire_latency {
    println!("p50={} p95={} p99={} p999={} mean={}ms",
        lat.p50_ms, lat.p95_ms, lat.p99_ms, lat.p999_ms, lat.mean_ms);
}
```

Latency is recorded via an [HDR histogram](https://hdrhistogram.github.io/HdrHistogram/)
with 1ms precision up to 30s. The histogram is stored in the pool state and
**not** reset between calls to `stats()` — it accumulates over the lifetime
of the pool.

---

## Auto-Scaling

`AutoScaler` watches pool utilisation and calls `scale_up` / `scale_down`
closures when thresholds are crossed. It is purely advisory — the pool still
enforces `max_size`.

### `AutoScalePolicy`

```rust
pub struct AutoScalePolicy {
    /// Utilisation (active / max_size) above which to scale up. Default: 0.8.
    pub high_watermark: f64,
    /// Utilisation below which to scale down. Default: 0.2.
    pub low_watermark: f64,
    /// Number of connections to add per scale-up step. Default: 2.
    pub scale_up_step: usize,
    /// Number of connections to remove per scale-down step. Default: 1.
    pub scale_down_step: usize,
    /// How often to evaluate utilisation. Default: 30s.
    pub evaluation_window: Duration,
    /// Minimum time between consecutive scale actions. Default: 60s.
    pub cooldown: Duration,
}
```

### Enabling auto-scaling via `ManagerBuilder`

```rust
// Apply to every pool registered with this manager:
let manager = ManagerBuilder::new()
    .default_autoscale_policy(AutoScalePolicy {
        high_watermark: 0.75,
        low_watermark: 0.25,
        scale_up_step: 3,
        ..Default::default()
    })
    .build();
```

### Enabling auto-scaling per resource

```rust
let key = ResourceKey::try_from("postgres")?;
manager.enable_autoscaling(&key, AutoScalePolicy::default())?;
```

The `AutoScaler` spawns a Tokio task that polls stats at `evaluation_window`
intervals. It respects the `cooldown` to avoid thrashing. The task is
cancelled when the `Manager` shuts down.

---

## Lifecycle of an Instance

```
Resource::create(config, ctx)
  │
  ▼ [instance enters pool]
  │
  ├─ idle in VecDeque
  │    │ idle_timeout exceeded → Resource::cleanup (CleanupReason::IdleTimeout)
  │    │ max_lifetime exceeded → Resource::cleanup (CleanupReason::Expired)
  │    └─ validation_interval  → Resource::is_reusable
  │         └─ false           → Resource::cleanup (CleanupReason::RecycleFailed)
  │
  ├─ checked out (Guard held by caller)
  │    │ guard.taint() called  → Resource::cleanup (CleanupReason::Tainted)
  │    └─ guard dropped        → Resource::recycle → back to idle queue
  │
  └─ Manager::shutdown()       → Resource::cleanup (CleanupReason::Shutdown)
```

---

## Configuration Examples

### Minimal (defaults)

```rust
PoolConfig::default()
// min_size=1, max_size=10, acquire_timeout=30s, Fifo, BoundedWait
```

### High-throughput API backend

```rust
PoolConfig {
    min_size: 5,
    max_size: 50,
    acquire_timeout: Duration::from_secs(5),
    idle_timeout: Duration::from_secs(300),
    max_lifetime: Duration::from_secs(3600),
    strategy: PoolStrategy::Lifo,
    backpressure_policy: Some(PoolBackpressurePolicy::Adaptive(
        AdaptiveBackpressurePolicy {
            high_pressure_utilization: 0.8,
            high_pressure_waiters: 10,
            low_pressure_timeout: Duration::from_secs(5),
            high_pressure_timeout: Duration::from_millis(200),
        },
    )),
    ..PoolConfig::default().with_standard_breakers()
}
```

### Single-tenant background worker

```rust
PoolConfig {
    min_size: 1,
    max_size: 3,
    acquire_timeout: Duration::from_secs(60),
    strategy: PoolStrategy::Fifo,
    backpressure_policy: Some(PoolBackpressurePolicy::BoundedWait {
        timeout: Duration::from_secs(60),
    }),
    ..PoolConfig::default()
}
```

### Fail-fast microservice sidecar

```rust
PoolConfig {
    min_size: 2,
    max_size: 10,
    backpressure_policy: Some(PoolBackpressurePolicy::FailFast),
    ..PoolConfig::default().with_standard_breakers()
}
```
