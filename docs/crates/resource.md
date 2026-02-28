# nebula-resource

Lifecycle management for long-lived resources: database connection pools, HTTP clients,
message queue connections, cache clients. Handles pooling, health checks, scope-aware
cleanup, and back-pressure.

## Core Traits

### `Resource`

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Instance: ResourceInstance;

    fn metadata(&self) -> ResourceMetadata;

    async fn create(
        &self,
        config: &Self::Config,
        ctx: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError>;

    fn validate_config(&self, config: &Self::Config) -> Result<(), ResourceError> {
        config.validate()
    }

    fn supports_pooling(&self) -> bool { true }
}
```

### `ResourceInstance`

```rust
pub trait ResourceInstance: Send + Sync + 'static {
    fn id(&self) -> &ResourceInstanceId;

    async fn health_check(&self) -> Result<HealthStatus, ResourceError>;

    /// Called when the instance is returned to the pool or destroyed.
    async fn cleanup(&mut self) -> Result<(), ResourceError> { Ok(()) }

    /// Reset state for pool reuse (e.g., rollback pending transactions).
    async fn reset(&mut self) -> Result<(), ResourceError> { Ok(()) }

    fn is_reusable(&self) -> bool { true }
}
```

## Lifecycle

Resources live within a **scope** (from `nebula-core`). When the scope ends, all resources
bound to it are automatically returned to the pool or destroyed.

```
deploy workflow
    │
    ▼
Global scope resources created (long-lived pools, shared clients)
    │
    ▼  workflow execution starts
Workflow scope resources created
    │
    ▼  node executes
ResourceAction::configure() → pool.acquire()
    │
    ▼  node finishes (or branch fails)
ResourceAction::cleanup(owned_instance) → pool.release()
    │
    ▼  execution ends
Execution scope resources destroyed
    │
    ▼  workflow undeployed
Global scope resources destroyed
```

## Module Structure

```
nebula-resource/src/
├── core/
│   ├── resource.rs        Resource + ResourceInstance traits
│   ├── metadata.rs        ResourceMetadata, CapabilityRequirements
│   ├── health.rs          HealthStatus, HealthChecker
│   └── error.rs           ResourceError
├── manager/
│   ├── manager.rs         ResourceManager — central registry
│   ├── registry.rs        Resource type registry
│   ├── lifecycle.rs       Scope-aware lifecycle hooks
│   ├── allocation.rs      Acquire / release logic
│   └── recovery.rs        Failure recovery
├── pool/
│   ├── pool.rs            Generic resource pool
│   ├── strategies.rs      FIFO, LIFO, least-loaded
│   └── metrics.rs         Pool utilization metrics
├── health/
│   ├── checker.rs         Periodic health checks
│   ├── aggregator.rs      Aggregate health across instances
│   └── recovery.rs        Auto-recovery actions
└── types/                 Built-in resource types
    ├── database.rs        Database connection pools (sqlx PgPool, etc.)
    ├── http_client.rs     HTTP clients (reqwest)
    ├── message_queue.rs   MQ connections
    └── cache.rs           Cache clients
```

## Registering a Resource

```rust
use nebula_resource::{Resource, ResourceConfig, ResourceInstance, ResourceManager};

pub struct DatabasePool;

impl Resource for DatabasePool {
    type Config = DatabaseConfig;
    type Instance = PgPool;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata {
            id: "database.postgres",
            name: "PostgreSQL Pool",
            category: ResourceCategory::Database,
            ..Default::default()
        }
    }

    async fn create(
        &self,
        config: &DatabaseConfig,
        _ctx: &ResourceContext,
    ) -> Result<PgPool, ResourceError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.url)
            .await
            .map_err(ResourceError::connection)?;
        Ok(pool)
    }
}

// Registration
let mut manager = ResourceManager::new();
manager.register::<DatabasePool>(DatabasePool);
```

## Acquiring Resources in Actions

```rust
// Global registry access (any action type)
let pool = ctx.resource::<PgPool>().await?;
let rows = sqlx::query("SELECT id FROM users").fetch_all(&pool).await?;
```

For branch-scoped resources use `ResourceAction` (see [action.md](./action.md)).

## Health Monitoring

Health checks run periodically in the background. Unhealthy instances are removed from the
pool and recreated.

```rust
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String, latency_p99: Duration },
    Unhealthy { reason: String },
}
```

Aggregated health is exposed via the REST API (`GET /resources/{id}/health`) and the
metrics endpoint.

## Back-Pressure

When all pool slots are occupied, `acquire()` parks the caller on a bounded internal queue.
If the queue fills, the call returns `ResourceError::PoolExhausted` immediately rather than
blocking indefinitely.

Configure via `PoolConfig`:

```rust
PoolConfig {
    min_size: 2,
    max_size: 20,
    acquire_timeout: Duration::from_secs(5),
    idle_timeout: Duration::from_secs(300),
    ..Default::default()
}
```

## Scope-Aware Cleanup

Resources registered under an execution scope are automatically released when the execution
ends — even if it fails or is cancelled:

```rust
let handle = manager
    .acquire_scoped::<HttpClient>(ScopeLevel::Execution(exec_id))
    .await?;

// handle is returned to pool when dropped or when the execution scope ends
```
