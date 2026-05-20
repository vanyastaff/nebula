# nebula-resource

Type-safe, topology-aware resource management for the Nebula workflow engine.
`nebula-resource` gives workflow nodes stable, managed access to external
clients — database connections, HTTP clients, message-queue producers, and
anything else that is costly to create and should be reused across executions.
It handles the full operational lifecycle: create → health-check → recycle →
shutdown → destroy, with credential rotation, recovery gating, and lifecycle
event streaming.

> **Maturity: `frontier`.** The public API still evolves between minor releases.

---

## Core Concepts

| Type | Role |
|------|------|
| [`Resource`] | Central trait — 4 associated types + lifecycle methods (`create`, `check`, `shutdown`, `destroy`) + slot-rotation hooks (`on_credential_refresh`, `on_credential_revoke`) |
| [`Pooled`] | Topology trait — N interchangeable instances with checkout/recycle |
| [`Resident`] | Topology trait — one shared instance cloned on each acquire |
| [`Bounded`] | Topology trait — parameterised on a sealed `Cap` typestate: `Unbounded`, `Capped<N>`, `Exclusive` |
| [`Manager`] | Central registry — single `register(RegistrationSpec { … })` funnel, typed acquire dispatch, slot rotation, graceful shutdown |
| [`ResourceGuard`] | RAII lease guard; releases on drop, tainting supported |
| [`ResourceContext`] | Execution context — scope, cancellation, capability traits |
| [`Error`] / [`ErrorKind`] | Unified error with retryability, scope, and optional retry-after hint |

---

## Topology Decision Guide

| Topology | Use when | Example |
|----------|----------|---------|
| `Pooled` | N interchangeable connections; checkout/recycle | PostgreSQL, Redis |
| `Resident` | One shared object; cheap to `Arc::clone` for each caller | `reqwest::Client`, in-memory cache |
| `Bounded<Unbounded>` | Long-lived runtime issuing short-lived tokens | OAuth client |
| `Bounded<Capped<N>>` | Multiplexed sessions over one shared connection | gRPC channel, AMQP |
| `Bounded<Exclusive>` | One caller at a time, semaphore(1) + reset-on-release | File lock, USB-serial |

The `Bounded` topology folded the former standalone `Service` / `Transport` /
`Exclusive` traits into one runtime; the `Cap` typestate selects concurrency
arity at **compile time** — using `BoundedRelease` against `Unbounded`, or
omitting it for `Capped` / `Exclusive`, is a build error.

> **Background workers and event sources** live in
> [`nebula-engine`](https://docs.rs/nebula-engine) (`nebula_engine::daemon::*`).
> They are not part of the `nebula-resource` topology surface.

---

## Quick Start

### 1. Implement `Resource`

```rust,no_run
use std::future::Future;
use nebula_core::ResourceKey;
use nebula_resource::{
    resource_key, Error, Resource, ResourceConfig, ResourceContext, HasSchema, ValidSchema,
};

#[derive(Clone, Hash)]
struct HttpConfig {
    base_url: String,
    timeout_ms: u64,
}

impl HasSchema for HttpConfig {
    fn schema() -> ValidSchema { ValidSchema::empty() }
}

impl ResourceConfig for HttpConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.base_url.is_empty() {
            return Err(Error::permanent("base_url must not be empty"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct HttpRuntime { base_url: String }

// No `#[credential]` field — this resource needs no credential. A credential-
// bound resource instead declares `#[credential(key = "...")] auth: SlotCell<CredentialGuard<C>>`.
struct HttpResource;

impl Resource for HttpResource {
    type Config = HttpConfig;
    type Runtime = HttpRuntime;
    type Lease = HttpRuntime;   // Pooled / Resident: Lease == Runtime
    type Error = Error;

    fn key() -> ResourceKey { resource_key!("http.client") }

    fn create(
        &self, config: &HttpConfig, _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<HttpRuntime, Error>> + Send {
        async move { Ok(HttpRuntime { base_url: config.base_url.clone() }) }
    }
}
```

### 2. Implement a topology trait

For the pool topology, implement [`Pooled`]:

```rust,ignore
use nebula_resource::topology::pooled::{Pooled, BrokenCheck, RecycleDecision, InstanceMetrics};

impl Pooled for HttpResource {
    fn is_broken(&self, _runtime: &HttpRuntime) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    async fn recycle(
        &self, _runtime: &HttpRuntime, _metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}
```

### 3. Register and acquire — the single funnel

Registration goes through **one funnel**:
`Manager::register::<R>(spec: RegistrationSpec<R>)`. There are no per-topology
`register_<topo>[_with]` shorthands — they were removed with the manager-side
`AcquireResilience` wrapper. Retry composes one layer up; per-topology configs
carry their own `create_timeout`.

```rust,ignore
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use nebula_core::{scope::Scope, ScopeLevel};
use nebula_resource::{
    AcquireOptions, Manager, PoolRuntime, RegistrationSpec, ResourceContext,
    SlotIdentity, TopologyRuntime,
    topology::pooled::config::Config as PoolConfig,
};

#[tokio::main]
async fn main() -> Result<(), nebula_resource::Error> {
    let manager = Manager::new();

    let config = HttpConfig { base_url: "https://api.example.com".into(), timeout_ms: 5_000 };
    let pool_rt = PoolRuntime::<HttpResource>::try_new(
        PoolConfig { max_size: 10, ..PoolConfig::default() },
        config.fingerprint(),
    )?;

    manager.register(RegistrationSpec {
        resource: HttpResource,
        config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Pool(pool_rt),
        acquire: Manager::erased_acquire_pooled_for::<HttpResource>(),
        recovery_gate: None,
    })?;

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let guard = manager
        .acquire_pooled::<HttpResource>(&ctx, &AcquireOptions::default())
        .await?;

    let _runtime: &HttpRuntime = &*guard;
    // Drop returns the instance to the pool via Pooled::recycle.
    // Call guard.taint() to skip recycle and force destroy.
    Ok(())
}
```

### 4. Attach a recovery gate (production)

For production, attach an `Arc<RecoveryGate>` via
`RegistrationSpec::recovery_gate` to prevent thundering-herd when a backend
flaps. The gate is the only manager-level resilience seam today; see
[`recovery.md`](recovery.md) for the state machine.

```rust,ignore
use nebula_resource::recovery::{RecoveryGate, RecoveryGateConfig};

let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register(RegistrationSpec {
    resource: DbResource,
    config: db_config,
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: TopologyRuntime::Pool(pool_rt),
    acquire: Manager::erased_acquire_pooled_for::<DbResource>(),
    recovery_gate: Some(gate),
})?;
```

---

## Error Handling

Every `register` / `acquire_*` call returns `Result<_, Error>`. The
`ErrorKind` enum drives retry decisions:

| Variant                     | Meaning                              | Retryable?           |
|-----------------------------|--------------------------------------|----------------------|
| `Transient`                 | Network blip, timeout                | Yes                  |
| `Permanent`                 | Auth failure, bad config             | No                   |
| `Exhausted { retry_after }` | Rate-limited or quota depleted       | Yes (after cooldown) |
| `Backpressure`              | Pool semaphore full                  | Caller decides       |
| `NotFound`                  | Key not in registry for that scope   | No                   |
| `Cancelled`                 | Cancellation token fired             | No                   |
| `Revoked`                   | Slot was revoked; retry after re-bind | Yes (after rotation) |
| `Ambiguous`                 | Multiple resolved identities; caller must pin | No           |

Use `err.is_retryable()` to branch without matching on variants; use
`err.retry_after()` to respect rate-limit hints.

### `#[derive(ClassifyError)]` — auto-`From<YourError>`

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

Supported kinds: `transient`, `permanent`, `exhausted` (with optional
`retry_after = "30s" / "5m" / "1h" / "500ms"`), `backpressure`, `cancelled`.

---

## Feature Matrix

| Capability                                  | How to enable                                                  |
|---------------------------------------------|---------------------------------------------------------------|
| Bounded connection pooling                  | `RegistrationSpec { topology: TopologyRuntime::Pool(_), .. }` |
| Shared singleton with clone-on-acquire      | `RegistrationSpec { topology: TopologyRuntime::Resident(_), .. }` |
| Long-lived runtime, short-lived tokens      | `RegistrationSpec { topology: TopologyRuntime::Bounded(_), .. }` with `Cap = Unbounded` |
| Multiplexed sessions over shared transport  | `Cap = Capped<N>` + `impl BoundedRelease`                     |
| Single-caller serialized access             | `Cap = Exclusive` + `impl BoundedRelease`                     |
| Fast-fail during backend recovery           | `RegistrationSpec::recovery_gate: Some(Arc<RecoveryGate>)`    |
| Config hot-reload (fingerprint-based)       | Implement `ResourceConfig::fingerprint`; call `Manager::reload_config` |
| Per-tenant credential isolation             | Build `SlotIdentity::from_bindings(…)` and acquire via `acquire_<topo>_for_identity` |
| Lifecycle event stream                      | `manager.subscribe_events()` → `broadcast::Receiver<ResourceEvent>` |
| Async background cleanup                    | `ReleaseQueue` (owned by `Manager`, transparent to callers)   |
| Atomic operation counters                   | `manager.metrics()` → `Option<&ResourceOpsMetrics>`           |

Retry/timeout on the acquire path composes one layer up (action handler /
engine activity). Per-topology configs carry their own `create_timeout` for
the create step.

---

## Crate Layout

```
crates/resource/
├── src/
│   ├── lib.rs              re-exports, crate-level docs
│   ├── resource.rs         Resource trait (4 associated types + lifecycle + slot-rotation hooks)
│   ├── slot.rs             SlotCell — public, generation-stamped, lock-free credential-slot holder
│   ├── cell.rs             internal lock-free cell (Resident runtime; not re-exported)
│   ├── manager/
│   │   ├── mod.rs              Manager + register/acquire + refresh_slot/revoke_slot (two-phase revoke)
│   │   ├── options.rs          ManagerConfig, RegisterOptions, RegistrationSpec, ShutdownConfig
│   │   ├── gate.rs             Recovery-gate admission helpers
│   │   ├── acquire_dispatch.rs Type-erased acquire factories
│   │   └── shutdown.rs         graceful_shutdown + drain helpers
│   ├── registry.rs         Registry, AnyManagedResource — type-erased storage
│   ├── guard.rs            ResourceGuard — RAII acquire lease (Owned / Guarded / Shared)
│   ├── context.rs          ResourceContext — execution context
│   ├── dedup.rs            SlotIdentity (Unbound / Structural), DedupKey
│   ├── error.rs            Error, ErrorKind, ErrorScope
│   ├── events.rs           ResourceEvent — 14 lifecycle event variants (all emitted)
│   ├── options.rs          AcquireOptions (deadline-only)
│   ├── metrics.rs          ResourceOpsMetrics, ResourceOpsSnapshot
│   ├── state.rs            ResourcePhase, ResourceStatus
│   ├── release_queue.rs    ReleaseQueue — background async cleanup workers
│   ├── reload.rs           ReloadOutcome (NoChange / SwappedImmediately)
│   ├── resource_ref.rs     ResourceRef — lazy reference type for action contexts
│   ├── topology_tag.rs     TopologyTag — Pool / Resident / Bounded discriminant
│   ├── ext.rs              HasResourcesExt (sealed)
│   ├── recovery/           RecoveryGate, RecoveryTicket, RecoveryWaiter, GateState
│   ├── runtime/            per-topology runtime wrappers + ManagedResource
│   └── topology/           per-topology trait definitions + Cap typestate
└── docs/
    ├── README.md           ← this file
    ├── api-reference.md    pointer to rustdoc + crate README prose surface
    ├── topology-reference.md  topology selection guide + minimal skeletons
    ├── adapters.md         writing an adapter crate (`nebula-resource-postgres` walkthrough)
    ├── pooling.md          PoolConfig deep-dive
    ├── recovery.md         RecoveryGate state machine
    └── events.md           ResourceEvent catalog
```

---

## Documentation

| Document                                          | Contents                                                                 |
|---------------------------------------------------|--------------------------------------------------------------------------|
| [`api-reference.md`](api-reference.md)            | Generated-rustdoc pointer + prose surface anchor                         |
| [`topology-reference.md`](topology-reference.md)  | Topology selection guide + minimal skeletons (Pool / Resident / Bounded) |
| [`adapters.md`](adapters.md)                      | Writing a `Resource` adapter crate                                       |
| [`pooling.md`](pooling.md)                        | `PoolConfig`, recycle, broken-check, max-lifetime                        |
| [`events.md`](events.md)                          | `ResourceEvent` catalog + `subscribe_events` patterns                    |
| [`recovery.md`](recovery.md)                      | `RecoveryGate` state machine                                             |
