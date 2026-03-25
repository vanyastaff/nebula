# nebula-resource

Type-safe, topology-agnostic resource management for the Nebula workflow engine.
Manages external resources (databases, HTTP clients, message brokers, etc.)
with a unified lifecycle: create, health-check, shutdown, destroy. Resources
declare a topology that determines how instances are pooled, shared, or isolated.

## Topologies

| Topology | Use case | Instance model |
|---|---|---|
| **Pooled** | Databases (Postgres, Redis) | N interchangeable instances with checkout/recycle |
| **Resident** | HTTP clients (`reqwest::Client`) | One shared instance, clone on acquire |
| **Service** | OAuth APIs, token-gated services | Long-lived runtime with short-lived tokens |
| **Transport** | WebSocket, gRPC channels | Shared connection with multiplexed sessions |
| **Exclusive** | File locks, hardware ports | One caller at a time via semaphore(1) |
| **EventSource** | Webhooks, SSE, change streams | Pull-based event subscription (secondary) |
| **Daemon** | Background workers, watchers | Background run loop with restart policy (secondary) |

## Action author view

```rust,ignore
// Inside an action's execute method:
let handle = ctx.resource::<PostgresPool>().await?;
let conn: &PgConnection = &*handle;
conn.query("SELECT 1").await?;
// handle dropped -> connection returned to pool
```

The `ResourceHandle` dereferences to the lease type and manages cleanup on drop.

## Resource author view

Implement the `Resource` trait plus a topology trait:

```rust,ignore
use nebula_resource::{Resource, resource_key};
use nebula_resource::topology::pooled::Pooled;

impl Resource for MyDatabase {
    type Config = DbConfig;       // operational config (no secrets)
    type Runtime = DbPool;        // the live connection pool
    type Lease = DbConnection;    // what callers hold
    type Error = DbError;         // resource-specific error
    type Credential = DbCreds;    // injected before create()

    fn key() -> ResourceKey { resource_key!("my-database") }

    async fn create(&self, config: &DbConfig, cred: &DbCreds, ctx: &dyn Ctx)
        -> Result<DbPool, DbError> { /* ... */ }

    async fn check(&self, runtime: &DbPool) -> Result<(), DbError> { /* ... */ }
    async fn shutdown(&self, runtime: &DbPool) -> Result<(), DbError> { /* ... */ }
    async fn destroy(&self, runtime: DbPool) -> Result<(), DbError> { /* ... */ }
}

impl Pooled for MyDatabase {
    fn is_broken(&self, runtime: &DbPool) -> BrokenCheck { /* sync O(1) check */ }
    async fn recycle(&self, runtime: &DbPool, metrics: &InstanceMetrics)
        -> Result<RecycleDecision, DbError> { /* keep or drop */ }
}
```

## Registration

```rust,ignore
let manager = Manager::new();

manager.register(
    MyDatabase::new(),
    db_config,
    db_creds,
    ScopeLevel::Global,
    TopologyRuntime::Pool(PoolRuntime::new(pool_config, fingerprint)),
    release_queue,
)?;

// Later, acquire:
let handle = manager.acquire_pooled::<MyDatabase>(&cred, &ctx).await?;
```

## Error classification

Every error carries an `ErrorKind` that determines retry behavior:

| Kind | Retryable | Example |
|------|-----------|---------|
| `Transient` | Yes | Network timeout, connection reset |
| `Exhausted` | Yes (after cooldown) | Rate limit, quota depleted |
| `Permanent` | No | Invalid config, auth failure |
| `Backpressure` | Caller decides | Pool full |
| `NotFound` | No | Resource key not registered |
| `Cancelled` | No | Shutdown in progress |

## Local verification

```bash
cargo fmt
cargo clippy -p nebula-resource -- -D warnings
cargo nextest run -p nebula-resource
cargo test -p nebula-resource --doc
```
