# nebula-resource Documentation

## For AI Agents

## Crate Purpose

`nebula-resource` is the **resource management framework** for Nebula. Provides lifecycle, pooling, scoping, and observability for databases, HTTP clients, message queues, etc.

## Core Concepts

### Resource
Any external system/service used by workflows (DB, API, cache, queue).

### ResourceManager
Central registry managing all resources with lifecycle, pooling, health checks.

### ResourceScope
Defines resource visibility and lifetime:
- `Global` - Shared across system
- `Tenant(id)` - Tenant-specific
- `Workflow(id)` - Workflow-level
- `Execution(id)` - Execution-specific

### ResourceContext
Carries execution context (tenant, user, workflow, execution IDs) for tracing and multi-tenancy.

## Key Traits

```rust
#[async_trait]
pub trait Resource: Send + Sync {
    type Config;
    type Instance;

    async fn create(&self, config: &Self::Config, context: &ResourceContext)
        -> ResourceResult<Self::Instance>;
}

pub trait HealthCheckable {
    async fn health_check(&self) -> ResourceResult<()>;
}

pub trait Poolable {
    async fn is_valid(&self) -> bool;
    async fn reset(&mut self) -> ResourceResult<()>;
}
```

## Integration Points

**Used By**: All workflow nodes/actions requiring external resources
**Uses**: `nebula-credential`, `nebula-error`, `nebula-resilience`

## When to Use

✅ Managing database connections
✅ HTTP client pooling
✅ Message queue producers/consumers
✅ Multi-tenant resource isolation
✅ Automatic health checking

## Common Patterns

### Register and Use
```rust
manager.register("db", PostgresResource).await?;
let db = manager.get::<PgConnection>("db").await?;
```

### With Pooling
```rust
let config = PoolConfig { min_size: 5, max_size: 20, .. };
manager.register_with_pool("db", PostgresResource, config).await?;
```

### Scoped Resources
```rust
let tenant_mgr = manager.scoped(ResourceScope::Tenant(tenant_id));
tenant_mgr.get::<Redis>("cache").await?; // Tenant-specific instance
```

## Feature Flags

- `postgres`, `mysql`, `mongodb`, `redis` - Database drivers
- `http-client` - HTTP resources
- `kafka` - Kafka support
- `credentials` - Credential integration
- `pooling` - Connection pooling
- `metrics` - Prometheus metrics
- `tracing` - OpenTelemetry tracing
- `testing` - Mock resources

## Performance

- Pooling: O(1) acquire/release
- Health checks: Configurable interval (default 60s)
- Connection reuse: Reduces overhead by 10-100x vs create-per-use

## Thread Safety

All types are `Send + Sync`. Manager uses `DashMap` for concurrent access.

## Version

See [Cargo.toml](./Cargo.toml)
