# Resource Author Contracts

> **Hard invariants that the Rust compiler cannot enforce.**
> Violating these contracts leads to subtle bugs: deadlocks, data corruption,
> silent stale resources, or process-wide OOM. The framework trusts resource
> authors to uphold them.

---

## 1. `Resident::is_alive_sync()` — O(1), no I/O, no blocking

**Contract:** This method runs inside an async runtime on a tokio worker thread.
It MUST complete in O(1) time with zero I/O. Acceptable: atomic flag checks,
`is_connected()` on clients that expose an atomic connectivity flag.

**Why:** Blocking a tokio worker thread stalls all tasks on that thread.
If `is_alive_sync()` does a network round-trip (e.g., Kafka `fetch_metadata`),
it blocks the entire thread pool under contention.

**If you need I/O health checks:** Implement `Resource::check()` instead (async).
The framework calls `check()` in appropriate contexts (pool `test_on_checkout`,
`CheckPolicy::Interval`, `WatchdogHandle`).

```rust
// CORRECT: O(1) atomic flag
fn is_alive_sync(&self, client: &fred::Client) -> bool {
    client.is_connected() // atomic load, ~1ns
}

// WRONG: network round-trip in sync context
fn is_alive_sync(&self, producer: &FutureProducer) -> bool {
    producer.client().fetch_metadata(None, Duration::from_secs(2)).is_ok() // BLOCKS!
}
```

---

## 2. `Pooled::is_broken()` — O(1), sync only

**Contract:** Called in the `Drop` path of `LeaseGuard` / `HandleInner::Guarded`.
Must be O(1) with no async, no I/O, no allocations on the hot path.
Checks local state only: TCP closed flag, error counter, process alive.

**Why:** `Drop` is synchronous. Blocking here delays the release pipeline and
can cause cascading backpressure across the pool.

**If unsure:** Return `BrokenCheck::NeedsAsyncCheck`. The pool will call
`Resource::check()` (async) before handing the instance to the next caller.

```rust
// CORRECT: local flag checks
fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
    if conn.client.is_closed() { BrokenCheck::Broken("TCP closed".into()) }
    else if conn.conn_task.is_finished() { BrokenCheck::Broken("conn task done".into()) }
    else { BrokenCheck::Healthy }
}

// WRONG: network I/O in Drop path
fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
    match conn.client.simple_query("SELECT 1") { ... } // BLOCKS in Drop!
}
```

---

## 3. `ResourceConfig::fingerprint()` — hash all compatibility-affecting fields

**Contract:** `fingerprint()` must return a stable hash of all config fields that
affect instance compatibility. If two configs produce different fingerprints, existing
instances are considered stale and will be evicted at next recycle.

**When `0` is correct:**
- `HttpConfig` (stateless client, no per-instance compatibility concerns)
- Configs where reload = full destroy + recreate (no incremental stale detection needed)

**When `0` is a bug:**
- `PgResourceConfig` with `statement_timeout`, `search_path` — changing these makes
  existing connections incompatible. `fingerprint()` MUST hash them.
- Any config with fields that affect connection behavior after creation.

**Why it matters:** `fingerprint() = 0` permanently disables stale detection for this
resource type. Config changes will silently not propagate to existing instances.

```rust
// CORRECT: hash compatibility-affecting fields
fn fingerprint(&self) -> u64 {
    let mut h = DefaultHasher::new();
    self.statement_timeout.hash(&mut h);
    self.application_name.hash(&mut h);
    self.search_path.hash(&mut h);
    h.finish()
}

// CORRECT: stateless, no stale semantics
fn fingerprint(&self) -> u64 { 0 } // HttpConfig — OK

// BUG: fields that affect instance behavior are not hashed
fn fingerprint(&self) -> u64 { 0 } // PgResourceConfig — WRONG
```

---

## 4. `Service::TOKEN_MODE::Tracked` requires `release_token()` implementation

**Contract:** If `TOKEN_MODE = TokenMode::Tracked`, the `release_token()` method
MUST be implemented. The default noop is only valid for `TokenMode::Cloned`.

**Why:** Tracked tokens are wrapped in `HandleInner::Guarded` with an `on_release`
callback that invokes `release_token()`. If `release_token()` is a noop, the tracking
resource (semaphore permit, rate limiter slot) is never returned, causing resource
exhaustion.

```rust
// CORRECT: Tracked token with proper release
const TOKEN_MODE: TokenMode = TokenMode::Tracked;

async fn release_token(&self, runtime: &Self::Runtime, permit: Self::Lease) -> Result<(), Self::Error> {
    drop(permit); // returns semaphore permit
    Ok(())
}

// BUG: Tracked mode with default (noop) release
const TOKEN_MODE: TokenMode = TokenMode::Tracked;
// release_token: using default noop — PERMITS LEAK!
```

---

## 5. `Transport` sessions should be bounded

**Contract:** Transport resources should configure `max_sessions` in their topology
config. Without a bound, a burst of concurrent `open_session()` calls can exhaust
the transport's multiplexing capacity (SSH channel limit, AMQP channel limit).

**How:** Framework enforces via `Arc<Semaphore>` with `max_sessions` permits.
Resource authors set this in `transport::Config`. Amendment #21 in 08-correctness.md.

**Default behavior without bound:** Unbounded session creation until the transport
connection fails or the remote server rejects new channels.

---

## 6. `Daemon::run()` must respect `CancellationToken`

**Contract:** The `run()` method MUST check the provided `CancellationToken` and
exit promptly when cancelled. The framework uses this token for graceful shutdown.

**Why:** If `run()` ignores the token, the daemon cannot be stopped gracefully.
`ShutdownOrchestrator` will timeout and forcefully abort the task, potentially
leaving resources in an inconsistent state.

```rust
// CORRECT: respect cancellation
async fn run(&self, runtime: &Self::Runtime, _ctx: &dyn Ctx, cancel: CancellationToken)
    -> Result<(), Self::Error>
{
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = self.poll_loop(runtime) => result,
    }
}

// BUG: ignores cancellation
async fn run(&self, runtime: &Self::Runtime, _ctx: &dyn Ctx, _cancel: CancellationToken)
    -> Result<(), Self::Error>
{
    loop { self.do_work(runtime).await?; } // NEVER STOPS
}
```

---

## 7. `ResourceHandle::detach()` — advanced API, use with care

**Contract:** `detach()` transfers ownership of the lease from the pool to the caller.
After detach:
- The pool does NOT track this instance anymore (no recycle, no destroy, no metrics).
- The caller is responsible for all cleanup (including closing connections).
- Pool accounting (active count, size) reflects the detached instance as "gone".
- `taint()` has no effect after detach (no release callback to notify).

**When safe:**
- Long-running SSH sessions that outlive the normal acquire/release cycle.
- Detached transactions that the caller manages explicitly.
- Migration: moving a connection from one pool to another.

**When almost certainly wrong:**
- Detaching from Pool and never closing the connection (connection leak).
- Detaching from Resident or Shared handles (`DetachError::NotDetachable`).
- Using detach to "work around" pool timeout — fix the timeout instead.

---

## Summary table

| Contract | Enforced by | Violation consequence |
|----------|-------------|----------------------|
| `is_alive_sync()` O(1) no-I/O | Documentation only | Tokio thread pool starvation |
| `is_broken()` O(1) sync | Documentation only | Release pipeline backpressure |
| `fingerprint()` hash correctness | Documentation only | Silent stale instances |
| Tracked `release_token()` | Documentation only | Token/permit exhaustion |
| Transport `max_sessions` | Framework semaphore | Unbounded sessions on transport |
| Daemon `CancellationToken` | Framework timeout | Ungraceful shutdown |
| `detach()` cleanup responsibility | Ownership transfer | Connection leak |
