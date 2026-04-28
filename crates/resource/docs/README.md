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
| [`ResourceGuard`] | RAII lease guard; releases or recycles on drop, tainting supported |
| [`ResourceContext`] | Execution context: scope level, cancellation token, capability traits |
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

> **Background workers and event sources** live in
> [`nebula-engine`](https://docs.rs/nebula-engine) (`nebula_engine::daemon::*`)
> per [ADR-0037](../../../docs/adr/0037-daemon-eventsource-engine-fold.md).
> They are not part of the `nebula-resource` topology surface.

---

## Quick Start

### 1. Implement `Resource`

```rust,no_run
use nebula_resource::{
    resource_key, ResourceContext, Error, Manager, PoolConfig, Resource,
    ResourceConfig, ResourceMetadata,
};
use nebula_credential::NoCredential;
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
    type Credential = NoCredential;  // No credential needed

    fn key() -> ResourceKey {
        resource_key!("http.client")
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _scheme: &(),
        _ctx: &ResourceContext,
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
use nebula_resource::{Manager, PoolConfig, ResourceContext, AcquireOptions};
use nebula_core::scope::Scope;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), nebula_resource::Error> {
    let manager = Manager::new();

    // Simple registration — Credential = NoCredential, scope = Global, no resilience.
    manager.register_pooled(
        HttpResource,
        HttpConfig { base_url: "https://api.example.com".into(), timeout_ms: 5_000 },
        PoolConfig::default(),
    )?;

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());

    // acquire_pooled_default: no scheme arg for Credential = NoCredential.
    let handle = manager
        .acquire_pooled_default::<HttpResource>(&ctx, &AcquireOptions::default())
        .await?;

    // Use via Deref — handle is held until dropped.
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

#[derive(Debug, thiserror::Error, ClassifyError)]
pub enum DbError {
    #[error("connection lost: {0}")]
    #[classify(transient)]
    ConnectionLost(String),
    #[error("auth failed: {0}")]
    #[classify(permanent)]
    AuthFailed(String),
    #[error("rate limited")]
    #[classify(exhausted, retry_after = "30s")]
    RateLimited,
}
```

This generates `impl From<DbError> for nebula_resource::Error` with
correct `ErrorKind::Transient`, `ErrorKind::Permanent`, and
`ErrorKind::Exhausted { retry_after: Some(Duration::from_secs(30)) }`
respectively. The `retry_after` value supports `s` / `m` / `h` / `ms`
suffixes.

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
| Atomic operation counters | `Option<ResourceOpsMetrics>` via `manager.metrics()` (`None` when no registry configured) |

---

## Crate Layout

```
crates/resource/
├── src/
│   ├── lib.rs             Re-exports and crate-level docs
│   ├── resource.rs        Resource trait (5 associated types + 6 lifecycle methods)
│   ├── manager/           Manager directory — split per Tech Spec §5.4:
│   │   ├── mod.rs              Manager type + register/acquire entry points
│   │   ├── options.rs          ManagerConfig, RegisterOptions, ShutdownConfig, DrainTimeoutPolicy
│   │   ├── registration.rs     register_inner + reverse-index population
│   │   ├── gate.rs             Recovery-gate admission helpers
│   │   ├── execute.rs          Resilience pipeline + register-time pool config validation
│   │   ├── rotation.rs         ResourceDispatcher + on_credential_* fan-out
│   │   └── shutdown.rs         graceful_shutdown + drain helpers + set_phase_all*
│   ├── registry.rs        Registry, AnyManagedResource — type-erased storage
│   ├── guard.rs           ResourceGuard — RAII acquire lease (Owned / Guarded / Shared)
│   ├── context.rs         ResourceContext — execution context with capabilities
│   ├── error.rs           Error, ErrorKind, ErrorScope, RotationOutcome
│   ├── events.rs          ResourceEvent — 12 lifecycle event variants
│   ├── options.rs         AcquireOptions (deadline-only since R-051)
│   ├── metrics.rs         ResourceOpsMetrics, ResourceOpsSnapshot, OutcomeCountersSnapshot
│   ├── state.rs           ResourcePhase, ResourceStatus
│   ├── cell.rs            Cell — ArcSwap-based lock-free cell for Resident topology
│   ├── release_queue.rs   ReleaseQueue — background async cleanup workers
│   ├── reload.rs          ReloadOutcome
│   ├── topology_tag.rs    TopologyTag — 5-variant discriminant (post-ADR-0037)
│   ├── integration.rs     AcquireResilience, AcquireRetryConfig
│   ├── ext.rs             HasResourcesExt
│   ├── recovery/          RecoveryGate, RecoveryGroupRegistry, WatchdogHandle
│   ├── runtime/           Per-topology runtime wrappers (5 topologies)
│   └── topology/          Per-topology trait definitions (Pooled / Resident / Service / Transport / Exclusive)
└── docs/
    ├── README.md          ← this file
    ├── api-reference.md   Full public API with signatures
    ├── adapters.md        Implementing Resource for a driver crate
    ├── pooling.md         PoolConfig, recycle policy, broken-check, max-lifetime
    ├── events.md          ResourceEvent catalog, subscribe_events usage
    └── recovery.md        RecoveryGate, WatchdogHandle, gate state transitions
```

---

## Documentation

| Document | Contents |
|----------|----------|
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures |
| [`adapters.md`](adapters.md) | Writing a `Resource` adapter crate (`nebula-resource-postgres`, etc.) |
| [`pooling.md`](pooling.md) | `PoolConfig`, recycle decisions, broken checks, max-lifetime eviction |
| [`events.md`](events.md) | `ResourceEvent` catalog, `subscribe_events` patterns |
| [`recovery.md`](recovery.md) | `RecoveryGate`, `WatchdogHandle`, gate state transitions |
