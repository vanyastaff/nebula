# nebula-resource — Pooling

`Pooled<R>` is the runtime side of the pool topology (`crate::runtime::pool`,
re-exported as `nebula_resource::Pooled`). It manages a bounded set of
`R::Instance`s with semaphore-controlled access, idle/lifetime expiry,
FIFO/LIFO checkout ordering, configurable warmup, and optional per-checkout
health probes over the framework-owned `InstanceStore`. The author-facing
hook trait is `PoolProvider` (`crate::topology::pooled`); the configuration
type is `PoolConfig` (`crate::topology::pooled::config::Config`).

This page is a prose field reference — for the trait shape and a runnable
skeleton, see [`topology-reference.md`](topology-reference.md#pool); for the
full derive → register → acquire flow, see the doctest on `Manager::register`.

---

## How the pool works

**Acquire** (`Manager::acquire_pooled`, or `acquire_pooled_for_identity` for
credential-bound routes):

1. Non-blocking concurrency gate (`Topology::try_reserve`) — a semaphore
   permit, bounded by `PoolConfig::max_size`.
2. Pop from the idle queue (`Lifo`: most-recently-used / `Fifo`: oldest, per
   `PoolConfig::strategy`).
3. If `test_on_checkout` is `true`, run `Provider::check`; discard on `Err`.
4. If no idle instance survived, call `Provider::create` — bounded by
   `max_concurrent_creates` so a burst of concurrent acquires cannot fan out
   into unbounded parallel creates against a fragile backend.
5. Wrap the instance in a `ResourceGuard` (release callback attached).

**Release** (`ResourceGuard` drop):

- Tainted (`guard.taint()` called) → skip recycle, `Provider::destroy` via
  the `ReleaseQueue`.
- Otherwise → `PoolProvider::recycle` decides `Keep` (pushed back to the idle
  queue, subject to the revoke-epoch fence) or `Drop` (destroyed).
- A resource that declares `#[credential]` slots is **discarded by default**
  on release rather than re-pooled (cross-lease state bleed prevention) —
  override `recycle` to wipe per-lease state and return `Keep` to actually
  pool a credentialed connection.

**Background maintenance** (`maintenance_interval`): sweeps the idle queue;
instances past `idle_timeout` or `max_lifetime` are evicted via
`Provider::destroy`. `max_lifetime` eviction uses a small per-entry jitter
(`[0.95 × max_lifetime, max_lifetime]`, drawn once at creation) so a warmup
cohort created in the same instant does not all expire on the same
maintenance tick.

---

## `PoolConfig` field reference

| Field | Default | Effect |
|-------|---------|--------|
| `min_size` | `1` | The pool's warmup target — see [Warmup strategy](#warmup-strategy). |
| `max_size` | `10` | Hard cap on instances (idle + checked out). Acquires beyond this wait on the semaphore. Rejected at construction if `0` (would deadlock the checkout semaphore) — use `Pooled::try_new` on any config sourced from operator/JSON input. |
| `idle_timeout` | `Some(5 min)` | Evicts instances idle longer than this. `None` disables idle eviction. |
| `max_lifetime` | `Some(30 min)` | Evicts instances older than this (± the jitter band above). `None` disables. |
| `create_timeout` | `30 s` | Per-call timeout for `Provider::create`. |
| `strategy` | `Lifo` | `Lifo` (hot working set) or `Fifo` (even spread) — see [Idle selection strategy](#idle-selection-strategy). |
| `warmup` | `None` (no eager warmup) | Pre-create instances at pool startup — see [Warmup strategy](#warmup-strategy). |
| `test_on_checkout` | `false` | If `true`, runs `Provider::check` on every checkout; `Err` discards and recreates. |
| `maintenance_interval` | `30 s` | Background sweep interval for idle/lifetime eviction. |
| `max_concurrent_creates` | `3` | Caps concurrent `Provider::create` calls during cold-start / warmup. |

`PoolConfig::default()` is a sensible starting point; tune `min_size` /
`max_size` / `max_lifetime` against your backend's connection budget and TTL.
`Pooled::try_new` (the registration-path constructor) rejects
`min_size > max_size` or `max_size == 0` as a typed `Error::permanent` rather
than deadlocking on first acquire; `Pooled::new` asserts the same invariants
for compile-time-known configs (doctests, fixtures).

---

## Idle selection strategy

`Lifo` (default) keeps a small "hot" working set active and lets excess idle
instances expire naturally via `idle_timeout` — good when `min_size` is much
smaller than `max_size` and load is bursty (e.g. a PostgreSQL pool behind
PgBouncer). `Fifo` distributes evenly across all pooled instances, preventing
any single connection from going stale under steady, near-full utilization.

---

## Warmup strategy

Controls how instances are created at pool startup, up to `min_size`:

- **None** (default) — first acquire pays the cold-start cost.
- **Sequential** — instances created back-to-back, on the first acquire.
  Predictable startup latency.
- **Parallel** — fastest warmup but spikes connection count; verify the
  backend tolerates it.
- **Staggered (interval)** — connection-rate-limited startup, for a backend
  with connection-rate caps.

`max_concurrent_creates` clamps `Parallel` and bounds `Staggered`'s active
count regardless of the chosen strategy.

---

## Test on checkout

`test_on_checkout: true` runs `Provider::check` on every checkout and
discards+recreates on `Err`. Costs one round-trip per acquire; use it when
`PoolProvider::is_broken` (sync, no I/O) cannot detect TCP-half-open or
server-side timeouts cheaply, or when the cost of a failed downstream call
is much higher than a connection-test ping. For most adapters, prefer
letting the cheap synchronous `is_broken` check (runs in `Drop`) handle
obvious closures and skip `test_on_checkout`.

---

## `PoolStats`

`Manager::pool_stats::<R>(scope)` returns a point-in-time snapshot
(`idle`, `capacity`, `available_permits`, `in_use`). For aggregate
cross-pool counters, use `Manager::metrics()` (`ResourceOpsMetrics` /
`ResourceOpsSnapshot`).

---

## See also

- [`topology-reference.md`](topology-reference.md#pool) — trait skeleton, registration shape, friction points.
- [`recovery.md`](recovery.md) — thundering-herd protection for a flapping backend.
- The crate-root "Tuning" rustdoc section — the full config-knob table across all topologies.
