# Architecture

Nebula is a modular workflow engine built on Rust 1.93+. The workspace contains **26 crates**
organized in strict architectural layers with enforced one-way dependency direction.

## Layer Diagram

```
┌──────────────────────────────────────────────────────────┐
│                Presentation / API Layer                   │
│               api · apps/desktop (Tauri) · webhook        │
├──────────────────────────────────────────────────────────┤
│                  Developer Tools Layer                    │
│                     sdk · macros                          │
├──────────────────────────────────────────────────────────┤
│                    Execution Layer                        │
│              engine · runtime (queue, sandbox)            │
├──────────────────────────────────────────────────────────┤
│                 Business Logic Layer                      │
│         action · resource · credential · plugin           │
├──────────────────────────────────────────────────────────┤
│                      Core Layer                           │
│    core · workflow · execution · memory · expression      │
│               parameter · validator                       │
├──────────────────────────────────────────────────────────┤
│              Cross-Cutting Concerns Layer                 │
│      config · log · system · resilience · telemetry       │
├──────────────────────────────────────────────────────────┤
│                  Infrastructure Layer                     │
│                       storage                             │
└──────────────────────────────────────────────────────────┘
```

**Rules:** Higher layers depend on lower layers. No upward or circular dependencies.
Cross-cutting crates (`config`, `log`, `resilience`) may be imported at any layer.

## Crate Reference

### Core Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/core` | `nebula-core` | Identifiers (`ExecutionId`, `WorkflowId`, `NodeId`), scope system, shared traits |
| `crates/workflow` | `nebula-workflow` | Workflow definition types, graph model |
| `crates/execution` | `nebula-execution` | Execution state machine and types |
| `crates/memory` | `nebula-memory` | In-memory state, arenas, LRU/TTL caching |
| `crates/expression` | `nebula-expression` | Expression evaluation on `serde_json::Value` |
| `crates/parameter` | `nebula-parameter` | Parameter schema, type descriptors, builder API |
| `crates/validator` | `nebula-validator` | Validation combinator library |

### Business Logic Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/action` | `nebula-action` | `Action` trait, execution context, node output |
| `crates/resource` | `nebula-resource` | Resource lifecycle, scopes, health, pooling |
| `crates/credential` | `nebula-credential` | Encrypted credential storage (AES-256-GCM), rotation engine |
| `crates/plugin` | `nebula-plugin` | Plugin discovery and dynamic loading |

#### Credential–Resource Integration

`nebula-credential` and `nebula-resource` are wired together via a typed, event-driven contract:

- **Typed references**: `CredentialRef<C>` (backed by `CredentialId` + `PhantomData<C>`) and `ErasedCredentialRef` in `ResourceComponents` declare which credential instance and protocol type a resource requires.
- **Resource components**: Resources that need credentials implement `HasResourceComponents` and return a `ResourceComponents` value describing the bound credential and any sub-resources.
- **Credential-aware pools**: `Pool<R>` in `nebula-resource` can store serialized credential state (`serde_json::Value`) plus a `CredentialHandler<R::Instance>`, and exposes `handle_rotation()` to apply new state using the configured `RotationStrategy` (`HotSwap`, `DrainAndRecreate`, `Reconnect`).
- **Rotation events**: `CredentialManager` emits `CredentialRotationEvent` on every successful rotation; `Resource::Manager` subscribes via `rotation_subscriber()` and dispatches rotations to affected pools based on `CredentialId`.
- **Authorization hook**: Concrete resources implement `CredentialResource` and are wired to pools via `TypedCredentialHandler<I>`, which deserializes state and calls `I::authorize()` after `create()` and on rotation.

### Execution Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/engine` | `nebula-engine` | Workflow scheduling, DAG traversal |
| `crates/runtime` | `nebula-runtime` | Trigger management, workflow lifecycle, task queue (MemoryQueue), sandbox (InProcessSandbox) |

### Cross-Cutting Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/config` | `nebula-config` | Configuration loading, hot-reload |
| `crates/log` | `nebula-log` | Structured logging, tracing spans |
| `crates/system` | `nebula-system` | Cross-platform utilities, memory pressure detection |
| `crates/resilience` | `nebula-resilience` | Circuit breaker, retry policies, rate limiting |
| `crates/telemetry` | `nebula-telemetry` | Metrics, distributed tracing |

### Infrastructure Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/storage` | `nebula-storage` | Storage abstraction (key-value backends), WorkflowRepo, ExecutionRepo |

### API / Application Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/api` | `nebula-api` | REST + WebSocket server (axum) |
| `apps/desktop` | — | **Desktop app (Tauri)** — React + TypeScript UI, Rust backend; auth, workflows, monitor. Replaces former egui-based `nebula-app`. |
| `crates/webhook` | `nebula-webhook` | Inbound webhook ingestion |

### Developer Tools Layer

| Dir | Package | Responsibility |
|-----|---------|----------------|
| `crates/sdk` | `nebula-sdk` | All-in-one developer SDK and testing utilities |
| `crates/macros` | `nebula-macros` | Procedural macros: `#[node]`, `#[action]` |

### First-Party Plugins

| Dir | Package | Notes |
|-----|---------|-------|
| `plugins/github` | `nebula-plugin-github` | GitHub integration |
| `plugins/telegram` | `nebula-plugin-telegram` | Telegram integration |

## Data Flow

```
Trigger Event
      │
      ▼
  Runtime                   (detects trigger, creates execution)
      │
      ▼
  Engine                    (schedules DAG, resolves dependencies)
      │
      ▼
  Work Queue                (bounded mpsc, back-pressure)
      │
      ▼
  Sandbox                   (executes Action in isolation)
      │
      ├──► Node Output       (passed to downstream nodes)
      └──► Side Effects      (storage writes, log events)
```

## Value Model

All workflow data is represented as `serde_json::Value`. There is no separate value crate.

```rust
use serde_json::Value;

// Node output
let output: Value = json!({ "status": 200, "body": "OK" });

// Parameter access
let url = params.get("url").and_then(Value::as_str)?;
let timeout = params.get("timeout").and_then(Value::as_i64).unwrap_or(30);
```

## API Surface

Nebula exposes two API surfaces via `nebula-api`:

- **REST** — CRUD for workflows, executions, credentials, resources; OpenAPI spec generated.
- **WebSocket** — Real-time execution events and log streaming.

GraphQL is not planned.

## Async Patterns

```rust
// Prefer JoinSet for scoped concurrent tasks
let mut set = JoinSet::new();
set.spawn(async { /* work */ });

// Always support cancellation
tokio::select! {
    result = do_work() => Ok(result?),
    _ = shutdown.cancelled() => Err(Error::Cancelled),
}
```

**Channel conventions:**
| Use case | Channel type |
|----------|-------------|
| Work queues | `mpsc` (bounded) |
| Status broadcasts | `broadcast` |
| Request/response | `oneshot` |
| Shared mutable state | `RwLock` |

**Default timeouts:** HTTP 10 s · Database 5 s · General 30 s
