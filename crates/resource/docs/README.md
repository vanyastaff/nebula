# nebula-resource

Type-safe, topology-aware resource management for the Nebula workflow engine.
`nebula-resource` gives workflow nodes stable, managed access to external
clients — database connections, HTTP clients, message-queue producers, and
anything else that is costly to create and should be reused across executions.
It handles the full operational lifecycle: create → health-check → recycle →
shutdown → destroy, with built-in resilience, recovery gating, and lifecycle
event streaming.

---

## Core Concepts

| Type | Role |
|------|------|
| [`Resource`] | Central trait — 5 associated types, 4 lifecycle methods (`create`, `check`, `shutdown`, `destroy`) |
| [`Pooled`] | Topology trait for N interchangeable instances with checkout/recycle semantics |
| [`Resident`] | Topology trait for one shared instance cloned on each acquire |
| [`Service`] | Topology trait for a long-lived runtime that issues short-lived tokens |
| [`Transport`] | Topology trait for a shared connection with multiplexed sessions |
| [`Exclusive`] | Topology trait for serialized single-caller access via semaphore |
| [`Manager`] | Central registry — registration, typed acquire dispatch, graceful shutdown |
| [`ResourceHandle`] | RAII lease handle; releases or recycles on drop, tainting supported |
| [`Ctx`] / [`BasicCtx`] | Execution context: scope level, cancellation token, extensions |
| [`Error`] / [`ErrorKind`] | Unified error with retryability, scope, and optional retry-after hint |

---

## Topology Decision Guide

Choose the topology that matches the resource's concurrency model.

| Topology | Use when | Example |
|----------|----------|---------|
| `Pooled` | Multiple interchangeable connections; checkout/recycle needed | PostgreSQL, Redis |
| `Resident` | One shared object; cheap to clone for each caller | In-memory cache, config store |
| `Service` | Runtime is long-lived; callers get short-lived tokens from it | OAuth client issuing access tokens |
| `Transport` | Single connection multiplexed across callers (no per-caller clone) | gRPC channel, AMQP connection |
| `Exclusive` | Only one caller may use the resource at a time | Rate-limited SMS gateway |
| `Daemon` | Background task with no direct callers (secondary topology) | Metrics flush loop |
| `EventSource` | Pull-based subscription stream (secondary topology) | Webhook ingestion tail |

---

## Quick Start

### 1. Implement `Resource`

```rust,no_run
use nebula_resource::{
    resource_key, BasicCtx, Ctx, Error, Manager, PoolConfig, Resource,
    ResourceConfig, ResourceMetadata,
};
use nebula_core::ResourceKey;

// --- Config (no secrets) -----------------------------------------------------

#[derive(Clone)]
struct HttpConfig {
    base_url: String,
    timeout_ms: u64,
}

impl ResourceConfig for HttpConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.base_url.is_empty() {
            return Err(Error::permanent("base_url must not be empty"));
        }
        Ok(())
    }
}

// --- Runtime (the live client) -----------------------------------------------

#[derive(Clone)]
struct HttpRuntime {
    base_url: String,
}

// --- Resource descriptor -----------------------------------------------------

struct HttpResource;

impl Resource for HttpResource {
    type Config     = HttpConfig;
    type Runtime    = HttpRuntime;
    type Lease      = HttpRuntime;   // Pooled: Lease == Runtime (cloned on checkout)
    type Error      = Error;
    type Credential = ();            // No secrets needed

    fn key() -> ResourceKey {
        resource_key!("http.client")
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _credential: &(),
        _ctx: &dyn Ctx,
    ) -> Result<HttpRuntime, Error> {
        Ok(HttpRuntime { base_url: config.base_url.clone() })
    }
}
```

### 2. Implement the topology trait

For pool topology, implement [`Pooled`] to tell the runtime when an instance
can be recycled and how to detect a broken one.

```rust,ignore
use nebula_resource::topology::pooled::{Pooled, BrokenCheck, RecycleDecision, InstanceMetrics};

impl Pooled for HttpResource {
    fn is_broken(&self, _runtime: &HttpRuntime) -> BrokenCheck {
        BrokenCheck::Healthy       // HTTP clients don't break between uses
    }

    fn recycle(
        &self, _runtime: &HttpRuntime, _metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, HttpError>> + Send {
        async { Ok(RecycleDecision::Keep) }  // always reusable
    }
}
```

### 3. Register and acquire

```rust,ignore
use nebula_resource::{Manager, PoolConfig, BasicCtx, AcquireOptions, ExecutionId};

#[tokio::main]
async fn main() -> Result<(), nebula_resource::Error> {
    let manager = Manager::new();

    // Simple registration — credential = (), scope = Global, no resilience
    manager.register_pooled(
        HttpResource,
        HttpConfig { base_url: "https://api.example.com".into(), timeout_ms: 5_000 },
        PoolConfig::default(),
    )?;

    let ctx = BasicCtx::new(ExecutionId::new());

    // acquire_pooled_default: no credential noise for Credential = ()
    let handle = manager
        .acquire_pooled_default::<HttpResource>(&ctx, &AcquireOptions::default())
        .await?;

    // Use via Deref — handle is held until dropped
    let _runtime: &HttpRuntime = &*handle;

    // Instance recycled automatically on drop.
    // Call handle.taint() before dropping to skip recycle and force destroy.
    Ok(())
}
```

### 4. Register with resilience

For production use, add timeout + retry + recovery gate via `RegisterOptions`:

```rust,ignore
use nebula_resource::{
    Manager, PoolConfig, RegisterOptions, AcquireResilience, RecoveryGate, RecoveryGateConfig,
};
use std::sync::Arc;

let manager = Manager::new();
let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register_pooled_with(
    DbResource,
    db_config,
    PoolConfig { max_size: 20, ..Default::default() },
    RegisterOptions {
        resilience: Some(AcquireResilience::standard()),
        recovery_gate: Some(gate),
        ..Default::default()
    },
)?;
```

---

## Error Handling

Every `acquire_*` and `register_*` call returns `Result<_, Error>`. The
`ErrorKind` enum drives retry decisions:

| Variant | Meaning | Retryable? |
|---------|---------|-----------|
| `Transient` | Network blip, timeout | Yes |
| `Permanent` | Auth failure, bad config | No |
| `Exhausted { retry_after }` | Rate-limited or quota depleted | Yes (after cooldown) |
| `Backpressure` | Pool semaphore full | Caller decides |
| `NotFound` | Key not in registry | No |
| `Cancelled` | Cancellation token fired | No |

Use `err.is_retryable()` to branch without matching on variants. Use
`err.retry_after()` to respect rate-limit hints.

### ClassifyError derive macro

Use `#[derive(ClassifyError)]` to auto-generate `From<YourError> for Error`
with the correct `ErrorKind` per variant — no manual `From` impl needed:

```rust,ignore
use nebula_resource::ClassifyError;

#[derive(Debug, ClassifyError)]
pub enum DbError {
    #[classify(transient)]
    ConnectionLost(String),
    #[classify(permanent)]
    AuthFailed(String),
    #[classify(exhausted, retry_after_secs = 30)]
    RateLimited,
}
```

This generates the `impl From<DbError> for nebula_resource::Error` with
correct `ErrorKind::Transient`, `ErrorKind::Permanent`, and
`ErrorKind::Exhausted { retry_after: Some(30s) }` respectively.

---

## Feature Matrix

| Capability | How to use |
|------------|-----------|
| Bounded connection pooling | `register_pooled` + `PoolConfig` |
| Shared singleton with clone-on-acquire | `register_resident` + `ResidentConfig` |
| Long-lived runtime, short-lived tokens | `register_service` + `ServiceConfig` |
| Single-caller serialized access | `register_exclusive` + `ExclusiveConfig` |
| Multiplexed sessions over shared transport | `register_transport` + `TransportConfig` |
| Retry + timeout on acquire | `register_*_with` + `RegisterOptions { resilience: Some(...) }` |
| Fast-fail during backend recovery | `register_*_with` + `RegisterOptions { recovery_gate: Some(...) }` |
| Config hot-reload (fingerprint-based) | Implement `ResourceConfig::fingerprint` |
| Lifecycle event stream | `manager.subscribe_events()` → `broadcast::Receiver<ResourceEvent>` |
| Async background cleanup | `ReleaseQueue` (owned by `Manager`, transparent to callers) |
| Atomic operation counters | `ResourceMetrics` via `manager.metrics()` |

---

## Crate Layout

```
crates/resource/
├── src/
│   ├── lib.rs             Re-exports and crate-level docs
│   ├── resource.rs        Resource trait, ResourceConfig, Credential, ResourceMetadata
│   ├── manager.rs         Manager, ManagerConfig, ShutdownConfig
│   ├── registry.rs        Registry, AnyManagedResource — type-erased storage
│   ├── handle.rs          ResourceHandle — RAII acquire lease
│   ├── ctx.rs             Ctx trait, BasicCtx, ScopeLevel, Extensions
│   ├── error.rs           Error, ErrorKind, ErrorScope
│   ├── events.rs          ResourceEvent — lifecycle observability
│   ├── options.rs         AcquireOptions, AcquireIntent
│   ├── metrics.rs         ResourceMetrics, MetricsSnapshot
│   ├── state.rs           ResourcePhase, ResourceStatus
│   ├── cell.rs            Cell — ArcSwap-based lock-free cell for resident topologies
│   ├── release_queue.rs   ReleaseQueue — background async cleanup workers
│   ├── topology_tag.rs    TopologyTag — discriminant enum
│   ├── integration.rs     AcquireResilience, AcquireRetryConfig, AcquireCircuitBreakerPreset
│   ├── recovery/          RecoveryGate, RecoveryGroupRegistry, WatchdogHandle
│   ├── runtime/           Per-topology runtime wrappers (pool, resident, service, …)
│   ├── topology/          Per-topology trait definitions (Pooled, Resident, Service, …)
│   └── compat.rs          Deprecated v1 aliases (Context, Scope) — will be removed
└── docs/
    ├── README.md                ← this file
    ├── architecture.md          Module map, data flow, design invariants
    ├── api-reference.md         Full public API with signatures
    ├── pooling.md               PoolConfig, recycle policy, broken-check, max-lifetime
    ├── events-and-hooks.md      ResourceEvent catalog, subscribe_events usage
    ├── health-and-quarantine.md RecoveryGate, WatchdogHandle, GateState transitions
    └── adapters.md              Implementing Resource for a driver crate
```

---

## Documentation

| Document | Contents |
|----------|----------|
| [`architecture.md`](architecture.md) | Module dependency map, data flow, layer invariants |
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures |
| [`pooling.md`](pooling.md) | `PoolConfig`, recycle decisions, broken checks, max-lifetime eviction |
| [`events-and-hooks.md`](events-and-hooks.md) | `ResourceEvent` catalog, `subscribe_events` patterns |
| [`health-and-quarantine.md`](health-and-quarantine.md) | `RecoveryGate`, `WatchdogHandle`, gate state transitions |
| [`adapters.md`](adapters.md) | Writing a `Resource` adapter crate (`nebula-resource-postgres`, etc.) |
