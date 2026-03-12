# Architecture

Nebula is a modular workflow engine built on Rust 1.93+. The workspace contains **26 crates** organized in strict architectural layers with enforced one-way dependency direction.

---

## Layer Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                  Presentation / API Layer                     │
│            api · apps/desktop (Tauri) · webhook               │
├──────────────────────────────────────────────────────────────┤
│                   Developer Tools Layer                       │
│                      sdk · macros                             │
├──────────────────────────────────────────────────────────────┤
│                     Execution Layer                           │
│                    engine · runtime                           │
├──────────────────────────────────────────────────────────────┤
│                  Business Logic Layer                         │
│            action · resource · credential · plugin            │
├──────────────────────────────────────────────────────────────┤
│                      Core Layer                               │
│     core · workflow · execution · memory · expression         │
│                  parameter · validator                        │
├──────────────────────────────────────────────────────────────┤
│                Cross-Cutting Concerns Layer                   │
│   config · log · system · resilience · telemetry             │
│              eventbus · metrics                               │
├──────────────────────────────────────────────────────────────┤
│                   Infrastructure Layer                        │
│                        storage                                │
└──────────────────────────────────────────────────────────────┘
```

**Rules:**
- Higher layers depend on lower layers. No upward or circular dependencies.
- Cross-cutting crates (`config`, `log`, `resilience`, `eventbus`) may be imported at any layer.
- `nebula-core` is the only crate that may be imported by every other crate.

---

## Crate Reference

### Core Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/core` | `nebula-core` | IDs (`ExecutionId`, `WorkflowId`, `NodeId`), scope hierarchy, shared traits |
| `crates/workflow` | `nebula-workflow` | Workflow definition types, DAG graph model |
| `crates/execution` | `nebula-execution` | Execution state machine and transition types |
| `crates/memory` | `nebula-memory` | In-memory state, arenas, LRU/TTL caching, memory pressure detection |
| `crates/expression` | `nebula-expression` | Expression evaluation on `serde_json::Value` |
| `crates/parameter` | `nebula-parameter` | Parameter schema, type descriptors, builder API |
| `crates/validator` | `nebula-validator` | Validation combinator library |

### Business Logic Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/action` | `nebula-action` | `Action` trait, execution context, output/error/port contracts |
| `crates/resource` | `nebula-resource` | Resource lifecycle, scopes, health checks, connection pooling |
| `crates/credential` | `nebula-credential` | Encrypted credential storage (AES-256-GCM), rotation engine |
| `crates/plugin` | `nebula-plugin` | Plugin discovery and dynamic loading |

#### Credential–Resource Integration

`nebula-credential` and `nebula-resource` are wired via a typed, event-driven contract:

- **Typed references**: `CredentialRef<C>` and `ErasedCredentialRef` in `ResourceComponents` declare which credential a resource requires.
- **Rotation events**: `CredentialManager` emits `CredentialRotationEvent`; `Resource::Manager` subscribes and dispatches rotations to affected pools based on `CredentialId`.
- **Rotation strategies**: `HotSwap`, `DrainAndRecreate`, `Reconnect` — selected per resource type.
- **Authorization hook**: Concrete resources implement `CredentialResource`; pools call `I::authorize()` after `create()` and on rotation.

### Execution Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/engine` | `nebula-engine` | Workflow scheduling, DAG traversal, node execution orchestration |
| `crates/runtime` | `nebula-runtime` | Action runner, task queue (`MemoryQueue`), in-process sandbox (`InProcessSandbox`), data passing policy |

### Cross-Cutting Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/config` | `nebula-config` | Configuration loading, hot-reload |
| `crates/log` | `nebula-log` | Structured logging, tracing spans |
| `crates/system` | `nebula-system` | Cross-platform utilities, memory pressure detection |
| `crates/resilience` | `nebula-resilience` | Circuit breaker, retry policies, rate limiting |
| `crates/telemetry` | `nebula-telemetry` | Distributed tracing, observability integration |
| `crates/eventbus` | `nebula-eventbus` | Pub/sub event bus (planned abstraction over current per-crate channels) |
| `crates/metrics` | `nebula-metrics` | Metrics collection and Prometheus/OTLP export |

### Infrastructure Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/storage` | `nebula-storage` | Key-value storage abstraction; `MemoryStorage` now, PostgreSQL planned |

### API / Application Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/api` | `nebula-api` | REST + WebSocket server (axum); workflow CRUD, execution events |
| `apps/desktop` | — | Tauri desktop app — React + TypeScript UI, Rust backend; auth, workflow editor, monitor |
| `apps/web` | — | Web frontend (browser version of the desktop app) |
| `crates/webhook` | `nebula-webhook` | Inbound webhook ingestion and routing |

### Developer Tools Layer

| Crate | Package | Responsibility |
|-------|---------|----------------|
| `crates/sdk` | `nebula-sdk` | All-in-one developer SDK, testing utilities, prelude |
| `crates/macros` | `nebula-macros` | Procedural macros: `#[node]`, `#[action]` |

---

## Data Flow

```
Trigger Event (webhook / cron / manual)
      │
      ▼
  Runtime                   detects trigger, creates Execution
      │
      ▼
  Engine                    loads Workflow, builds ExecutionContext, schedules DAG
      │
      ▼
  Runtime                   resolves ActionHandler from registry, enforces DataPassingPolicy
      │
      ▼
  Sandbox (InProcess)        executes Action in isolation with capability-checked Context
      │
      ├──► NodeOutput        passed to downstream nodes via expression evaluation
      └──► Side Effects      storage writes, EventBus events, telemetry spans
```

---

## Value Model

All workflow data is `serde_json::Value`. There is no separate value crate.

```rust
use serde_json::{Value, json};

// Node output
let output: Value = json!({ "status": 200, "body": "OK" });

// Parameter access (type-safe helpers)
let url = params.get("url").and_then(Value::as_str)?;
let timeout = params.get("timeout").and_then(Value::as_i64).unwrap_or(30);
```

This keeps serialization boundaries simple: JSON in at the trigger, JSON out to storage and downstream nodes.

---

## API Surface

Two API surfaces via `nebula-api`:

- **REST** — CRUD for workflows, executions, credentials, resources. OpenAPI spec generated.
- **WebSocket** — Real-time execution events and log streaming.

GraphQL is not planned (see [DECISIONS.md](./DECISIONS.md)).

---

## Async Conventions

```rust
// Prefer JoinSet for scoped concurrent tasks
let mut set = JoinSet::new();
set.spawn(async { /* work */ });

// Always wire cancellation
tokio::select! {
    result = do_work() => Ok(result?),
    _ = shutdown.cancelled() => Err(Error::Cancelled),
}
```

**Channel conventions:**

| Use case | Channel |
|----------|---------|
| Work queues | `mpsc` (bounded) |
| Status broadcasts | `broadcast` |
| Request/response | `oneshot` |
| Shared mutable state | `RwLock` |

**Default timeouts:** HTTP 10 s · Database 5 s · General 30 s

---

## Key Dependency Chains

```
nebula-core
  ├── nebula-workflow, nebula-execution, nebula-parameter, nebula-validator
  │     └── nebula-action
  │           └── nebula-engine ──────────────────────→ nebula-api
  │                 └── nebula-runtime → sandbox (in-process)
  │
  ├── nebula-storage ─────────────── needed by engine, credential
  ├── nebula-resource ─────────────┐
  └── nebula-credential ──────────→┤ credential–resource integration
                                   └── nebula-action (context capabilities)
```

Cross-cutting crates (`log`, `config`, `resilience`, `metrics`, `telemetry`, `eventbus`) are depended on by crates at any layer and do not depend on business logic.

---

## Future Crates (Planned, Not Yet in Workspace)

These concepts are documented and planned but do not yet exist as workspace members:

| Concept | Likely crate | Phase |
|---------|-------------|-------|
| PostgreSQL resource adapter | `nebula-resource-postgres` | Phase 2 |
| Sandbox isolation (WASM/process) | `nebula-sandbox` | Phase 2+ |
| Distributed worker pool | `nebula-worker` | Phase 3 |
| Idempotency keys | `nebula-idempotency` | Phase 3 |
| Multi-tenancy | `nebula-tenant` | Phase 4 |
| Cluster coordination | `nebula-cluster` | Phase 5 |
| Localization | `nebula-locale` | Phase 5 |

These are **not** breaking the current architecture — they slot into existing layers.
