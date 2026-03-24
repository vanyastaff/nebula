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
    let mut h = FxHasher::default(); // stable cross-process (not SipHash)
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

## 8. `Exclusive::reset()` — must be idempotent and handle partial state

**Contract:** `reset()` is called between callers to restore the resource to a clean
state. The previous caller may have been dropped mid-operation (panic, cancel, timeout).
`reset()` must handle any state the resource could be in — not just "completed normally".

**Why:** If `reset()` assumes the previous caller finished cleanly, partial state
(uncommitted offsets, half-written buffers, open cursors) will leak to the next caller.
If `reset()` fails, framework falls back to destroy + recreate.

```rust
// CORRECT: handle any state
async fn reset(&self, consumer: &StreamConsumer) -> Result<(), KafkaError> {
    // Commit whatever offsets exist (even if previous caller didn't finish)
    consumer.commit_consumer_state(CommitMode::Sync)?;
    // Clear any pending fetched messages
    consumer.pause_all();
    consumer.resume_all();
    Ok(())
}

// BUG: assumes previous caller committed
async fn reset(&self, consumer: &StreamConsumer) -> Result<(), KafkaError> {
    // Nothing to do — caller already committed
    Ok(()) // WRONG: if caller was cancelled, offsets are uncommitted
}
```

---

## 9. `Pooled::recycle()` — must not panic

**Contract:** `recycle()` runs inside a `ReleaseQueue` worker task. A panic in
`recycle()` aborts the worker, and all pending release tasks on that worker are lost.
This causes connection leaks and pool exhaustion.

**Why:** ReleaseQueue workers are shared — one panic affects all subsequent releases
assigned to that worker. `catch_unwind` is not used (async + Send constraints).

**Rule:** All error paths in `recycle()` must return `Err(...)` or `Ok(RecycleDecision::Drop)`,
never panic. Use `?` for fallible operations. If `DISCARD ALL` fails — return `Drop`,
don't unwrap.

```rust
// CORRECT: all errors handled
async fn recycle(&self, conn: &PgConnection, _metrics: &InstanceMetrics)
    -> Result<RecycleDecision, PgError>
{
    match conn.client.simple_query("DISCARD ALL").await {
        Ok(_) => Ok(RecycleDecision::Keep),
        Err(_) => Ok(RecycleDecision::Drop), // don't unwrap, don't panic
    }
}

// BUG: unwrap in recycle
async fn recycle(&self, conn: &PgConnection, _: &InstanceMetrics)
    -> Result<RecycleDecision, PgError>
{
    conn.client.simple_query("DISCARD ALL").await.unwrap(); // PANICS on closed conn!
    Ok(RecycleDecision::Keep)
}
```

---

## 10. `Resource::create()` — cancellation safety

**Contract:** `create()` may be cancelled via `CancellationToken` or `tokio::time::timeout`.
If cancellation occurs mid-create, no zombie resources should remain (open connections,
spawned tasks, allocated server-side resources).

**Why:** Pool warmup, acquire retry, and shutdown all may cancel in-flight `create()` calls.
If `create()` spawns a tokio task before returning and then gets cancelled, that task
leaks forever.

**Rules:**
- Do not `tokio::spawn()` before the final `Ok(runtime)` return.
- If you must spawn (e.g., Postgres connection task), use a guard that aborts the task on drop.
- Server-side resources (created schema, allocated session) must be cleaned up if create fails.

```rust
// CORRECT: spawn only after successful setup, with abort-on-drop guard
async fn create(&self, config: &PgConfig, cred: &DatabaseCredential, _ctx: &dyn Ctx)
    -> Result<PgConnection, PgError>
{
    let (client, connection) = tokio_postgres::Config::new()
        .host(&cred.host).connect(NoTls).await?;

    // AbortOnDrop: if PgConnection is dropped, the connection task is aborted.
    let conn_task = tokio::spawn(async move {
        if let Err(e) = connection.await { tracing::warn!("pg conn error: {e}"); }
    });

    Ok(PgConnection { client, conn_task: AbortOnDrop::new(conn_task) })
}

// BUG: spawned task leaks if create() is cancelled between spawn and return
async fn create(&self, config: &Config, cred: &Cred, _ctx: &dyn Ctx)
    -> Result<MyRuntime, MyError>
{
    let handle = tokio::spawn(background_loop()); // SPAWNED
    let client = connect(&cred).await?;           // ← if THIS fails, handle leaks!
    Ok(MyRuntime { client, handle })
}
```

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
| `reset()` idempotency | Documentation only | Leaked state between callers |
| `recycle()` no-panic | Documentation only | Worker abort, pool exhaustion |
| `create()` cancel-safety | Documentation only | Zombie tasks/connections |
