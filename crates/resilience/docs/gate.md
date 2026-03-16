# nebula-resilience — Gate

`Gate` and `GateGuard` provide a cooperative shutdown barrier for groups of concurrent
tasks or request handlers. They mirror the `Gate`/`GateGuard` pattern from
[Neon's page server](https://github.com/neondatabase/neon) and are used internally by
`Pool<R>` in `nebula-resource`.

---

## Table of Contents

- [Problem Statement](#problem-statement)
- [Core Types](#core-types)
- [How It Works](#how-it-works)
- [Usage Patterns](#usage-patterns)
- [Shutdown Sequencing](#shutdown-sequencing)
- [Error Handling](#error-handling)
- [Integration with tokio Tasks](#integration-with-tokio-tasks)
- [Examples](#examples)

---

## Problem Statement

A server that processes concurrent requests must be able to shut down gracefully: stop
accepting new requests and wait until all in-flight requests finish before exiting.
`Gate` solves this without requiring tasks to poll a shared flag or coordinate through
a `Mutex`.

---

## Core Types

### `Gate`

The cooperative shutdown barrier. Cheap to `Clone` — all clones share the same
underlying state via `Arc`.

```rust
pub struct Gate { /* Arc<GateInner> */ }

impl Gate {
    pub fn new() -> Self;
    pub fn enter(&self) -> Result<GateGuard, GateClosed>;
    pub async fn close(&self);
    pub fn is_closed(&self) -> bool;
}

impl Clone for Gate { /* clones the Arc */ }
```

---

### `GateGuard`

RAII exit token. While a `GateGuard` is alive, the gate cannot fully drain. Returns
one semaphore permit on drop.

```rust
pub struct GateGuard { /* private */ }

impl Drop for GateGuard {
    fn drop(&mut self) {
        // Returns one permit to the gate's semaphore.
        // Logs a WARN if dropped while gate is already closing.
    }
}
```

---

### `GateClosed`

Error returned by `Gate::enter` when the gate is already closing or closed.

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("gate is closed — new enter() calls are rejected")]
pub struct GateClosed;
```

---

## How It Works

Internally `Gate` uses a Tokio `Semaphore` pre-loaded with `u32::MAX / 2` permits and
an `AtomicBool` marking the closing state.

**`enter()`** — non-blocking:
1. Check `AtomicBool::closing`. If true, return `Err(GateClosed)` immediately.
2. Call `semaphore.try_acquire()`. A successfully acquired permit is immediately
   `forget()`-ed so it is not automatically returned.
3. Return a `GateGuard` that holds an `Arc` to the internal state.

**`GateGuard::drop()`**:
1. Optionally warn if dropping during active close.
2. Call `semaphore.add_permits(1)` to return the forgotten permit.

**`close()`** — async:
1. Set `AtomicBool::closing = true` so new `enter()` calls fail immediately.
2. Attempt to acquire all `u32::MAX / 2` permits. Because existing guards hold `N`
   permits, this blocks until all `N` guards have been dropped.
3. Logs progress at `WARN` level every second while waiting.

---

## Usage Patterns

### Request handler loop

```rust
use nebula_resilience::gate::{Gate, GateClosed};

let gate = Gate::new();

// Somewhere in the server's accept loop:
loop {
    let connection = listener.accept().await?;
    let guard = match gate.enter() {
        Ok(guard) => guard,
        Err(GateClosed) => {
            // Server is shutting down; reject new connections.
            break;
        }
    };

    tokio::spawn(async move {
        let _guard = guard; // keeps gate open while handling request
        handle_connection(connection).await;
        // guard drops here, returning its permit
    });
}
```

### Shutdown handler

```rust
// On SIGTERM or Ctrl-C:
gate.close().await;
// At this point all spawned handlers have finished.
tracing::info!("all in-flight requests drained");
```

### Sharing a gate across clones

`Gate` is `Clone`. You can pass clones to different components that all participate in
the same shutdown group:

```rust
let server_gate = Gate::new();
let worker_gate = server_gate.clone(); // same underlying semaphore

// Workers use worker_gate.enter(); both sets of guards drain on close.
server_gate.close().await;
```

---

## Shutdown Sequencing

Typical production shutdown order for a service using `Gate`:

```
1. Stop accepting new requests (e.g. close listener socket or set flag).
2. gate.close().await  — waits for all in-flight handlers.
3. Flush logs / metrics.
4. Release resources (pool shutdown, DB disconnect).
5. Exit process.
```

`close()` is idempotent — calling it more than once is safe.

---

## Error Handling

`GateClosed` should be treated as a normal signal, not an unexpected error. It means
the service is in the process of shutting down and is no longer accepting work.

```rust
match gate.enter() {
    Ok(guard) => {
        // Proceed with work, holding the guard.
        let _ = guard;
    }
    Err(GateClosed) => {
        // Return 503 Service Unavailable, drop the connection, or log and exit.
    }
}
```

---

## Integration with tokio Tasks

Because `GateGuard` is `'static + Send`, it can be moved into `tokio::spawn`:

```rust
let guard = gate.enter()?;

tokio::spawn(async move {
    let _guard = guard; // drops when the spawned task finishes
    do_work().await;
});
```

If the spawned task panics, `GateGuard::drop()` still runs (Rust's panic unwinding
guarantees RAII destructors), so the permit is always returned.

---

## Examples

### Worker pool with cooperative drain

```rust
use nebula_resilience::gate::{Gate, GateClosed};
use std::sync::Arc;
use tokio::sync::mpsc;

let gate = Gate::new();
let (tx, mut rx) = mpsc::unbounded_channel::<WorkItem>();

// Workers
for _ in 0..num_cpus::get() {
    let gate = gate.clone();
    let mut rx = /* each worker has its own receiver */;
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            let guard = match gate.enter() {
                Ok(g) => g,
                Err(GateClosed) => break,
            };
            tokio::spawn(async move {
                let _guard = guard;
                process(item).await;
            });
        }
    });
}

// Graceful shutdown
drop(tx); // stop sending
gate.close().await; // drain all in-flight work
```

### Timeout-bounded drain

If you need to enforce a maximum drain time, race `close()` against a deadline:

```rust
use std::time::Duration;

let drain = tokio::time::timeout(Duration::from_secs(30), gate.close()).await;

if drain.is_err() {
    tracing::warn!("graceful drain timed out after 30s; forcing shutdown");
}
```
