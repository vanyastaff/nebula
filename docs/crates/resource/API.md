# API

## Public Surface

- stable APIs:
  - `Resource`, `Config`, `Context`, `Scope`, `Strategy`
  - `Manager`, `ManagerBuilder`, `Pool`, `PoolConfig`, `PoolStrategy`
  - `ResourceProvider`, `ResourceRef`
  - `ResourceMetadata` — display name, description, icon, tags (used by API + desktop UI)
  - `Error` and `Result`
- observability APIs (stable, used by nebula-api for `GET /resources`):
  - `Manager::list_status()` → `Vec<ResourceStatus>` *(target — Phase 2)*
  - `Manager::get_status(id)` → `Option<ResourceStatus>` *(target — Phase 2)*
  - `Manager::event_bus()` → `&Arc<EventBus>` — subscribe to live `ResourceEvent` stream
  - `EventBus::subscribe()` → `broadcast::Receiver<ResourceEvent>`
- experimental APIs:
  - autoscaling and some advanced observability behavior should be treated as evolving contracts.
- hidden/internal APIs:
  - `manager_guard`, `manager_pool` are internal orchestration details.

## Observability Read Model

### `ResourceStatus` *(target — Phase 2)*

Aggregate snapshot combining metadata, health, pool stats, and quarantine state.
Used by `nebula-api` to serve `GET /resources` and `GET /resources/:id`.

```rust
/// Snapshot of a resource's current state for external observers (API, UI).
#[derive(Debug, Clone, Serialize)]
pub struct ResourceStatus {
    /// Static metadata: id, name, description, icon, tags.
    pub metadata: ResourceMetadata,
    /// Current health state.
    pub health: HealthState,            // Healthy | Degraded | Unhealthy
    /// Pool utilisation counters.
    pub pool: PoolStats,                // idle, in_use, waiting, max_size
    /// Whether the resource is currently quarantined.
    pub quarantined: bool,
    /// Quarantine reason, if quarantined.
    pub quarantine_reason: Option<String>,
    /// Scope this resource is registered under.
    pub scope: Scope,
}
```

`Manager::list_status()` collects a snapshot from `self.pools`, `self.health_states`,
`self.quarantine`, and `self.metadata` in one call. No locks held across the snapshot.

### `ResourceEvent` — live stream

The `EventBus` emits events on every lifecycle transition:

| Event | Key fields | Meaning |
|-------|------------|---------|
| `HealthChanged { from, to }` | resource_id | Health state transition |
| `Acquired { wait_duration }` | resource_id | Instance handed to caller |
| `Released { usage_duration }` | resource_id | Instance returned to pool |
| `PoolExhausted { waiters }` | resource_id | Pool full, callers waiting |
| `Quarantined { reason }` | resource_id | Resource put in quarantine |
| `QuarantineReleased` | resource_id, recovery_attempts | Recovered |
| `Error { error }` | resource_id | Operation failed |
| `CleanedUp { reason }` | resource_id | Instance permanently removed |

**Streaming pattern (used by `nebula-api` SSE handler):**

```rust
let mut rx = manager.event_bus().subscribe();
// In axum SSE handler:
while let Ok(event) = rx.recv().await {
    let json = serde_json::to_string(&event)?;
    yield SseEvent::default().data(json);
}
```

Each SSE connection gets its own `broadcast::Receiver` — no shared mutable state.

## Usage Patterns

- register once at startup, acquire many times during execution.
- prefer typed acquire (`acquire_typed`) where call site has static resource type.
- use scoped registration (`register_scoped`) for tenant/workflow isolation.

## Minimal Example

```rust
use nebula_resource::{Config, Context, Manager, PoolConfig, Resource, Scope};

#[derive(Clone)]
struct DbConfig { dsn: String }

impl Config for DbConfig {
    fn validate(&self) -> nebula_resource::Result<()> {
        if self.dsn.is_empty() {
            return Err(nebula_resource::Error::configuration("dsn is empty"));
        }
        Ok(())
    }
}

struct DbResource;

impl Resource for DbResource {
    type Config = DbConfig;
    type Instance = String;

    fn id(&self) -> &str { "db.main" }

    async fn create(
        &self,
        cfg: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok(format!("conn:{}", cfg.dsn))
    }
}

# async fn run() -> nebula_resource::Result<()> {
let manager = Manager::new();
manager.register(DbResource, DbConfig { dsn: "postgres://...".into() }, PoolConfig::default())?;

let ctx = Context::new(Scope::Global, "wf-1", "exec-1");
let guard = manager.acquire("db.main", &ctx).await?;
let _conn = guard.as_any().downcast_ref::<String>().unwrap();
# Ok(()) }
```

## Advanced Example

```rust
use nebula_resource::{Context, Manager, ManagerBuilder, PoolConfig, Scope};

# async fn run() -> nebula_resource::Result<()> {
let manager = ManagerBuilder::new().build();

let scope = Scope::workflow_in_tenant("wf-orders", "tenant-a");
manager.register_scoped(MyResource, (), PoolConfig::default(), scope)?;

let ctx = Context::new(
    Scope::execution_in_workflow("exec-42", "wf-orders", Some("tenant-a".into())),
    "wf-orders",
    "exec-42",
);

let typed = manager.acquire_typed(MyResource, &ctx).await?;
let _instance = typed.get();
# Ok(()) }
# struct MyResource;
# impl nebula_resource::Resource for MyResource {
#   type Config = (); type Instance = String;
#   fn id(&self)->&str{"my"}
#   async fn create(&self,_:&(),_:&Context)->nebula_resource::Result<Self::Instance>{Ok("ok".into())}
# }
```

## Error Semantics

- retryable errors:
  - `PoolExhausted`, `Timeout`, `Unavailable { retryable: true }`, `CircuitBreakerOpen`.
- fatal errors:
  - structural misconfiguration (`Configuration`, `Validation`) and non-retryable unavailable states.
- validation errors:
  - `Config::validate` and `PoolConfig::validate` must fail fast before activation.

## Compatibility Rules

- what changes require major version bump:
  - scope containment semantics
  - acquire/release lifecycle guarantees
  - error variant meaning changes
  - hook/event schema contract breaks
- deprecation policy:
  - additive APIs first, at least one minor cycle before removal unless security-critical.
