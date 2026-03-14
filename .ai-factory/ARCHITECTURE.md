# Architecture: Layered Modular Monolith (Cargo Workspace)

## Overview

Nebula is structured as a **layered modular monolith** implemented as a Rust Cargo workspace.
Each layer is a set of purpose-scoped crates with strict one-way dependency rules enforced at
compile time via `cargo deny`. This pattern gives the benefits of strong module boundaries
(testability, independent versioning, clear ownership) with the operational simplicity of a
single deployable binary.

The architecture is enforced by the Rust type system — a dependency that violates the layer order
simply fails to compile. `cargo deny` provides a secondary gate in CI for supply-chain auditing.

## Decision Rationale

- **Project type:** Workflow automation engine — complex business logic (DAG scheduling,
  credential management, plugin lifecycle) with a need for long-term maintainability
- **Tech stack:** Rust 2024 workspace, Axum, Tokio — crates are the natural module unit
- **Key factor:** Compile-time enforcement of layer boundaries, zero-cost abstractions across
  module boundaries, and the ability to test each layer in isolation with `MemoryStorage` / mock
  injections

## Workspace Structure

```
nebula/
├── crates/                          # All library crates (nebula-<noun>)
│   │
│   ├── core/          (L1)          # IDs, scopes, shared traits — never grows
│   ├── validator/     (L1)          # Validation combinators — zero deps
│   ├── parameter/     (L1)          # Parameter schema (Field, Schema, providers)
│   ├── expression/    (L1)          # Expression evaluation on serde_json::Value
│   ├── memory/        (L1)          # Arena, LRU/TTL caching, memory pressure
│   ├── workflow/      (L1)          # Workflow/DAG definition types
│   ├── execution/     (L1)          # Execution state machine types
│   │
│   ├── log/           (L2)          # Structured tracing wrapper
│   ├── system/        (L2)          # Cross-platform utilities
│   ├── eventbus/      (L2)          # Pub/sub decoupling bus
│   ├── metrics/       (L2)          # Counters, histograms, gauges
│   ├── telemetry/     (L2)          # Distributed tracing
│   ├── config/        (L2)          # Configuration loading + hot-reload
│   ├── resilience/    (L2)          # Circuit breaker, retry, rate-limit
│   │
│   ├── credential/    (L3)          # Encrypted credential storage + rotation
│   ├── resource/      (L3)          # Resource lifecycle, pooling, health
│   ├── action/        (L3)          # Action trait + context + execution model
│   ├── plugin/        (L3)          # Plugin discovery + registration
│   │
│   ├── engine/        (L4)          # DAG scheduler + orchestration
│   ├── runtime/       (L4)          # Trigger lifecycle, task queue, sandbox
│   │
│   ├── storage/       (L5)          # KV/repo abstraction, MemoryStorage, Postgres
│   │
│   ├── api/           (L6)          # Axum REST + WebSocket server
│   ├── webhook/       (L6)          # Inbound webhook ingestion
│   ├── auth/          (L6)          # JWT Bearer validation, session management
│   ├── sdk/           (L6)          # Action-author SDK (re-exports action authoring types)
│   └── macros/        (L6)          # Proc-macros: #[node], #[action], derive
│
├── apps/
│   └── desktop/                     # Tauri v2 + React + TypeScript (excluded from workspace)
│       └── src-tauri/               # Shares underlying Rust crates with server
│
├── migrations/                      # PostgreSQL schema migrations (sqlx)
├── deploy/                          # Docker / Kubernetes stacks
└── docs/                            # Architecture, API, roadmap docs
```

## Layer Map

| Layer | Label | Crates | May Depend On |
|-------|-------|--------|---------------|
| Core | L1 | `core`, `validator`, `parameter`, `expression`, `memory`, `workflow`, `execution` | Nothing (L1 only) |
| Cross-Cutting | L2 | `log`, `system`, `eventbus`, `metrics`, `telemetry`, `config`, `resilience` | L1 only |
| Business Logic | L3 | `credential`, `resource`, `action`, `plugin` | L1 + L2 |
| Execution | L4 | `engine`, `runtime` | L1 + L2 + L3 |
| Infrastructure | L5 | `storage` | L1 + L2 |
| Interface / API | L6 | `api`, `webhook`, `auth`, `sdk`, `macros` | L1–L5 |

**Note:** L2 (Cross-Cutting) crates may be imported at any layer — they contain no business logic.
L5 may be imported directly by L4 and L6.

## Dependency Rules

```
L1 Core  ←──  L2 Cross-Cutting  ←──  L3 Business  ←──  L4 Execution
                                  └─── L5 Infrastructure ──────────┘
                                                   └──  L6 Interface
```

- ✅ Higher-numbered layers may import lower-numbered layers
- ✅ L2 (cross-cutting) may be imported at any layer (L1–L6)
- ✅ L5 (storage) may be imported by L4 (execution) and L6 (interface)
- ❌ Lower layers must NEVER import higher layers (no upward deps)
- ❌ Circular dependencies between any two crates are forbidden
- ❌ `nebula-core` must not import any other workspace crate
- ❌ Infrastructure details (Postgres-specific types, SQL) must not leak above L5
- ❌ L3 business crates must not directly import each other — use `nebula-eventbus`

## Layer/Module Communication

### Within a layer — direct crate import

```toml
# crates/action/Cargo.toml
[dependencies]
nebula-core = { workspace = true }      # L1
nebula-eventbus = { workspace = true }  # L2
nebula-credential = { workspace = true } # L3 sibling — avoid; use eventbus instead
```

### Across bounded layers — EventBus (preferred for cross-L3 signals)

Cross-bounded-context events (e.g. credential rotation → resource pool update) must go through
`nebula-eventbus` to avoid circular imports between L3 crates:

```rust
// In nebula-credential — publish rotation event
use nebula_eventbus::{EventBus, ScopedEvent};

bus.emit(ScopedEvent::new("credential.rotated", rotation_payload)).await?;
```

```rust
// In nebula-resource — subscribe independently
use nebula_eventbus::EventBus;

bus.subscribe("credential.rotated", |event| async move {
    resource_pool.refresh_credentials(event.payload()).await
}).await;
```

### L4 → L3 — dependency injection via Context

Actions never construct runtime-managed types; they receive injected context:

```rust
// crates/action/src/context.rs
pub struct Context {
    pub credentials: Arc<dyn CredentialAccessor + Send + Sync>,
    pub resources:   Arc<dyn ResourceProvider + Send + Sync>,
    pub logger:      Arc<dyn Logger + Send + Sync>,
    pub cancellation: CancellationToken,
}
```

## Crate Conventions

### lib.rs template

```rust
//! Crate-level doc comment.
//!
//! ## Quick Start
//! ...
//!
//! ## Core Types
//! ...

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// ── Public modules ──────────────────────────────────────────────────────────
pub mod error;
pub mod prelude;

// ── Re-exports ───────────────────────────────────────────────────────────────
pub use error::MyError;

// ── Private modules ──────────────────────────────────────────────────────────
mod internal;
```

### Error type template

```rust
// crates/<name>/src/error.rs
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MyError {
    #[error("operation failed: {message}")]
    OperationFailed { message: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl MyError {
    pub fn operation_failed(msg: impl Into<String>) -> Self {
        Self::OperationFailed { message: msg.into() }
    }
}

pub type MyResult<T> = Result<T, MyError>;
```

### ID types (nebula-core pattern)

```rust
// crates/core/src/id.rs
use domain_key::define_uuid;

define_uuid!(Workflow => WorkflowId);
define_uuid!(Execution => ExecutionId);
define_uuid!(Node => NodeId);
```

## Runtime Data Flow

```
Trigger / inbound event
        │
        ▼
  nebula-runtime          ← lifecycle management, task queue
        │
        ▼
  nebula-engine           ← DAG topological sort + node dispatch
        │
        ▼
  nebula-action           ← Action::execute(ctx) with injected capabilities
        │
        ├── nebula-credential (via CredentialAccessor)
        ├── nebula-resource   (via ResourceProvider)
        └── nebula-log        (structured span)
        │
        ▼
  nebula-storage          ← persist execution state + output
        │
        ▼
  nebula-eventbus         ← emit execution events for subscribers
        │
        ▼
  nebula-api / WebSocket  ← stream real-time logs + status to clients
```

## Key Principles

1. **Compile-time boundary enforcement.** Layer violations are build errors, not code review
   comments. `cargo deny` provides a secondary CI-level gate.
2. **`nebula-core` is the bedrock.** It is imported by every crate. It must stay small and stable.
   New IDs are allowed; new utilities or business logic are not.
3. **EventBus for cross-L3 signals.** When two L3 crates need to communicate, neither imports
   the other — both import `nebula-eventbus` and exchange typed events.
4. **Dependency injection over service locators.** Actions receive a `Context` struct; they never
   construct runtime-managed types themselves.
5. **`serde_json::Value` as universal runtime data.** No custom value enum. All workflow I/O flows
   as JSON; type intent is described by parameter schemas (`nebula-parameter`).
6. **Storage abstraction hides backend.** Upper crates use `nebula-storage` traits. `MemoryStorage`
   is valid for tests; Postgres is the production backend. No Postgres-specific SQL above L5.
7. **Phase-gated features.** Phase 2 = in-process sandbox (`InProcessSandbox`). WASM/OS isolation
   is Phase 3. Do not add isolation features in Phase 2 code paths.
8. **Security zones.** `nebula-credential`, `nebula-auth`, `nebula-storage` are high-scrutiny
   change zones. Credentials are always AES-256-GCM encrypted at rest; never stored plaintext.

## Anti-Patterns

- ❌ **Upward dependency** — `nebula-core` importing `nebula-action` (or any higher crate).
  Add the type to `nebula-core` or create a shared L2 abstraction.
- ❌ **L3-to-L3 direct import** — `nebula-credential` importing `nebula-resource`. Use EventBus.
- ❌ **Global service locators** — an action calling `CredentialManager::global()`. Use
  `Context::credentials()` injection.
- ❌ **SQL leaking above L5** — Postgres row types or `sqlx::query!` results appearing in L3/L4
  function signatures.
- ❌ **Business logic in `nebula-core`** — adding a `retry_with_backoff` helper to core just
  because it's convenient. Put it in `nebula-resilience` (L2).
- ❌ **Panicking in library crates** — use `Result` and `?`. Only tests and unreachable branches
  may use `.unwrap()` with an explanatory comment.
- ❌ **Phase 3 features in Phase 2** — adding OS-process isolation or WASM sandboxing to the
  current `InProcessSandbox` execution path.
- ❌ **Shared global state across tests** — use `MemoryStorage` and fresh instances per test to
  keep tests isolated and parallel-safe.

## Guardrails Checklist (before every PR)

```bash
cargo fmt --all -- --check          # formatting
cargo clippy --workspace -- -D warnings  # lints
cargo check --workspace --all-targets   # type check
cargo test --workspace              # all tests
cargo doc --no-deps --workspace     # docs build
cargo deny check                    # supply-chain + dep rules
```
