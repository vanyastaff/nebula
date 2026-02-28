# Technical Notes

Architecture decision records (ADRs) for Nebula.

---

## ADR-001 — Rust as the implementation language

**Decision:** Rust 1.93+ (MSRV tracked in `workspace.rust-version`).

**Rationale:**
- Zero-cost abstractions — no GC overhead in hot execution paths
- Memory safety without garbage collection
- Expressive type system enables compile-time workflow validation
- First-class async via Tokio; excellent ecosystem

**Alternatives considered:** Go (weaker type system), C++ (more footguns), Java/C# (GC overhead).

---

## ADR-002 — `serde_json::Value` as the universal value type

**Decision:** All workflow runtime data uses `serde_json::Value`. There is no separate
`nebula-value` crate (migration completed in `008-serde-value-migration`).

**Rationale:**
- Less code and fewer dependencies
- Standard serde ecosystem; every Rust library already knows how to handle it
- Expressions and parameters operate directly on `serde_json::Value`

**Usage pattern:**
```rust
use serde_json::Value;

// Reading from params
let url = params.get("url").and_then(Value::as_str)?;
let count = params.get("count").and_then(Value::as_i64).unwrap_or(10);

// Producing output
let output: Value = json!({ "status": 200, "items": items });
```

---

## ADR-003 — REST + WebSocket API (no GraphQL)

**Decision:** `nebula-api` exposes REST (CRUD) and WebSocket (real-time events). GraphQL
is not planned.

**Rationale:**
- REST + OpenAPI is sufficient for workflow and credential CRUD
- WebSocket covers real-time execution streaming
- Simpler to implement, maintain, and version
- GraphQL can be layered on later if demand justifies it

---

## ADR-004 — Per-crate error types

**Decision:** Each crate defines its own `Error` enum using `thiserror`. There is no shared
`nebula-error` crate.

**Rationale:**
- Avoids a dependency that everything must pull in
- Each crate can express exactly the errors it produces
- Boundary conversions are explicit (`#[from]` or manual `impl From`)

**Pattern:**
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("action execution failed: {0}")]
    Execution(String),

    #[error("resource unavailable: {id}")]
    ResourceUnavailable { id: String },

    #[error(transparent)]
    Storage(#[from] nebula_storage::Error),
}
```

---

## ADR-005 — Tokio async runtime with cancellation

**Decision:** All async code runs on Tokio. Long-running tasks carry a `CancellationToken`.

**Pattern:**
```rust
use tokio_util::sync::CancellationToken;

async fn run(shutdown: CancellationToken) -> Result<()> {
    tokio::select! {
        result = do_work() => result,
        _ = shutdown.cancelled() => Err(Error::Cancelled),
    }
}
```

**Channel conventions:**
| Use case | Channel |
|----------|---------|
| Work queues | `mpsc` bounded |
| Status fan-out | `broadcast` |
| Request/response | `oneshot` |
| Shared state | `RwLock` (prefer over `Mutex` for read-heavy) |

**Default timeouts:** HTTP 10 s · Database 5 s · General 30 s

---

## ADR-006 — Plugin system via dynamic libraries

**Decision:** Plugins are native Rust dynamic libraries loaded via `libloading` with
convention-based discovery.

**Rationale:**
- Native performance; no IPC overhead
- Familiar model for Rust developers

**Trade-offs:**
- ABI stability requires careful versioning
- WASM sandbox considered but overhead and capability restrictions are prohibitive for v1

---

## ADR-007 — Capability-based sandbox for Action execution

**Decision:** Actions execute inside a sandbox that grants capabilities explicitly (network,
filesystem, system commands). Denied by default.

**Rationale:**
- Prevents accidental or malicious side effects from third-party nodes
- Auditable at the manifest level

---

## ADR-008 — Workspace structure

All crates live under `crates/` with short directory names. Package names retain the
`nebula-` prefix (e.g. `crates/core` → package `nebula-core`). First-party plugins live
under `plugins/`.

Resolver `3` is used (Rust 2024 edition feature unification).
