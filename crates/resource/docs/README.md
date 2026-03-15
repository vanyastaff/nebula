# nebula-resource

Resource lifecycle management for the Nebula workflow engine.

`nebula-resource` gives workflow nodes stable, pooled access to expensive external
clients — database connections, HTTP clients, message-queue producers, and anything
else that is costly to create and should be reused. It handles the full operational
lifecycle: create → validate → recycle → health-check → quarantine → shutdown.

---

## Table of Contents

- [Core Concepts](#core-concepts)
- [Quick Start](#quick-start)
- [Feature Matrix](#feature-matrix)
- [Crate Layout](#crate-layout)
- [Documentation](#documentation)

---

## Core Concepts

| Concept | Description |
|---------|-------------|
| **`Resource`** | Trait that defines how to create, validate, recycle, and clean up one type of external resource. |
| **`Pool<R>`** | Bounded pool of `R::Instance` values with semaphore-guarded concurrency, idle-expiry, and circuit breakers. |
| **`Manager`** | Central registry. Holds one `Pool<R>` per registered resource type. Handles dependency-ordered startup and graceful shutdown. |
| **`Guard<T>`** | RAII handle to a checked-out instance. Returns the instance to the pool on drop; taints it for cleanup if marked unhealthy. |
| **`Scope`** | Containment level: `Global → Tenant → Workflow → Execution → Action → Custom`. Controls which callers may acquire which pool. |
| **`Context`** | Per-call execution context carrying `Scope`, `WorkflowId`, `ExecutionId`, cancellation token, and an optional telemetry recorder. |
| **`HealthState`** | `Healthy / Degraded / Unhealthy / Unknown`. The `HealthChecker` monitors instances and triggers quarantine transitions. |
| **`QuarantineManager`** | Isolates unhealthy resources with exponential-backoff recovery probes. |
| **`EventBus`** | Broadcasts `ResourceEvent` on every lifecycle transition. Upstream crates subscribe without coupling to internals. |
| **`HookRegistry`** | Pre/post hooks for acquire, release, create, and cleanup. Hooks can cancel create operations. |

---

## Quick Start

### Defining a resource

```rust
use nebula_core::ResourceKey;
use nebula_resource::{Config, Context, Resource, ResourceMetadata, Result};

// --- Config ------------------------------------------------------------------

#[derive(Clone)]
pub struct HttpConfig {
    pub base_url: String,
    pub timeout_secs: u64,
}

impl Config for HttpConfig {
    fn validate(&self) -> Result<()> {
        if self.base_url.is_empty() {
            return Err(nebula_resource::Error::configuration("base_url must not be empty"));
        }
        Ok(())
    }
}

// --- Instance ----------------------------------------------------------------

pub struct HttpClient {
    inner: reqwest::Client,
}

// --- Resource ----------------------------------------------------------------

pub struct HttpResource;

impl Resource for HttpResource {
    type Config   = HttpConfig;
    type Instance = HttpClient;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::build(
            ResourceKey::try_from("http.client").expect("valid key"),
            "HTTP Client",
            "Reusable reqwest client for outbound HTTP calls",
        )
        .tag("http")
        .build()
    }

    async fn create(&self, cfg: &HttpConfig, _ctx: &Context) -> Result<HttpClient> {
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
            .build()
            .map_err(|e| nebula_resource::Error::configuration(e.to_string()))?;
        Ok(HttpClient { inner })
    }

    async fn is_reusable(&self, instance: &HttpClient) -> Result<bool> {
        // reqwest clients are always reusable — skip extra validation
        let _ = instance;
        Ok(true)
    }
}
```

### Registering and acquiring

```rust
use nebula_resource::{Context, Manager, PoolConfig, Scope};

// Build a manager (defaults: 1024-event bus, 3-failure quarantine threshold)
let manager = Manager::new();

// Register the resource — non-async, returns a typed pool handle
manager.register(HttpResource, HttpConfig { base_url: "https://api.example.com".into(), timeout_secs: 10 }, PoolConfig::default())?;

// Build a call context
let ctx = Context::new(
    Scope::try_execution_in_workflow("exec-1", "wf-orders", Some("tenant-a".into()))?,
    nebula_resource::WorkflowId::new(),
    nebula_resource::ExecutionId::new(),
);

// Acquire an instance — returned in a RAII Guard
let key = nebula_core::ResourceKey::try_from("http.client")?;
let guard = manager.acquire(&key, &ctx).await?;

// Use via Deref — guard holds the instance until dropped
let _client: &HttpClient = &*guard;

// Instance is returned to the pool automatically when `guard` is dropped.
// Call `guard.taint()` before dropping to skip recycling and force cleanup.
```

### Subscribing to events

```rust
use nebula_resource::EventBus;
use std::sync::Arc;

let bus = Arc::new(EventBus::new(1024));
let mut rx = bus.subscribe();

tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        tracing::info!(?event, "resource lifecycle event");
    }
});
```

---

## Feature Matrix

| Feature | Type | Default |
|---------|------|---------|
| Bounded connection pooling | `Pool<R>` | always on |
| FIFO / LIFO idle selection | `PoolStrategy` | `Fifo` |
| Backpressure: fail-fast, bounded-wait, adaptive | `PoolBackpressurePolicy` | `BoundedWait(30s)` |
| Circuit breakers on `create` and `recycle` | `CircuitBreakerConfig` in `PoolConfig` | opt-in |
| Scope isolation | `Scope` + `Strategy` | `Hierarchical` |
| RAII taint-on-error guards | `Guard<T>` | always on |
| Structured health checking | `HealthChecker`, `HealthCheckable` | always on |
| Automatic quarantine + recovery | `QuarantineManager` | always on |
| Lifecycle event streaming | `EventBus`, `ResourceEvent` | always on |
| Pre/post acquire & release hooks | `HookRegistry`, `ResourceHook` | always on |
| Auto-scaling (utilization-based) | `AutoScaler`, `AutoScalePolicy` | opt-in |
| Credential binding + rotation | `HasResourceComponents` | opt-in |
| Telemetry recorder per call | `Recorder` (from `nebula-telemetry`) | opt-in |
| Latency histograms (HDR) | `LatencyPercentiles` in `PoolStats` | always on |

---

## Crate Layout

```
nebula-resource/
├── src/
│   ├── lib.rs             Re-exports, prelude
│   ├── resource.rs        Resource + Config traits
│   ├── lifecycle.rs       Lifecycle state machine (Created → … → Terminated)
│   ├── guard.rs           Guard<T> — RAII acquire handle
│   ├── context.rs         Context — scope, IDs, cancellation, recorder
│   ├── scope.rs           Scope enum + Strategy
│   ├── pool.rs            Pool<R> — bounded semaphore pool
│   ├── manager.rs         Manager + ManagerBuilder — registry and orchestration
│   ├── health.rs          HealthChecker, HealthCheckable, HealthState
│   ├── quarantine.rs      QuarantineManager, RecoveryStrategy
│   ├── events.rs          EventBus, ResourceEvent catalog
│   ├── hooks.rs           HookRegistry, ResourceHook trait
│   ├── autoscale.rs       AutoScaler, AutoScalePolicy
│   ├── metadata.rs        ResourceMetadata + builder
│   ├── reference.rs       ResourceRef, ErasedResourceRef, ResourceProvider
│   ├── error.rs           Error, ErrorCategory, FieldViolation
│   ├── poison.rs          Poison<T> — async-safe state guard
│   ├── components.rs      HasResourceComponents, TypedCredentialHandler
│   ├── instrumented.rs    InstrumentedGuard — telemetry-aware wrapper
│   ├── metrics.rs         MetricsCollector
│   ├── dependency_graph.rs (internal) topological startup/shutdown ordering
│   ├── manager_guard.rs   (internal) AnyGuard, TypedResourceGuard, ResourceHandle
│   └── manager_pool.rs    (internal) TypedPool, AnyPool, PoolEntry
└── docs/
    ├── README.md                ← this file
    ├── architecture.md          Design decisions, module map, data flow
    ├── api-reference.md         Complete public API reference
    ├── pooling.md               Pool configuration, strategies, backpressure, auto-scaling
    ├── health-and-quarantine.md Health pipeline, quarantine lifecycle
    ├── events-and-hooks.md      EventBus catalog, subscriptions, hook system
    └── adapters.md              Writing Resource adapter / driver crates
```

---

## Documentation

| Document | Contents |
|----------|----------|
| [`architecture.md`](architecture.md) | Module dependency map, data flow, design invariants, internal types |
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures and examples |
| [`pooling.md`](pooling.md) | `Pool`, `PoolConfig`, strategies, backpressure policies, auto-scaling |
| [`health-and-quarantine.md`](health-and-quarantine.md) | `HealthChecker`, health states, `QuarantineManager`, recovery |
| [`events-and-hooks.md`](events-and-hooks.md) | `EventBus`, full event catalog, subscriptions, `HookRegistry` |
| [`adapters.md`](adapters.md) | Implementing `Resource` for a driver crate (`nebula-resource-postgres`, etc.) |
