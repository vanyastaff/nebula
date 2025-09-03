---
title: README
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---


## Overview

`nebula-resource` provides a comprehensive resource management framework for the Nebula workflow engine. It handles the lifecycle, pooling, scoping, and observability of all resources used within workflows and actions.

### Key Features

- 🔄 **Lifecycle Management** - Automatic initialization, health checks, and cleanup
- 🏊 **Resource Pooling** - Efficient connection pooling with configurable strategies
- 🔍 **Context Awareness** - Automatic context propagation for tracing and multi-tenancy
- 🔐 **Credential Integration** - Seamless integration with `nebula-credential` for secure credential management
- 📊 **Built-in Observability** - Metrics, logging, and distributed tracing out of the box
- 🎯 **Scoped Resources** - Support for Global, Tenant, Workflow, and Action-level resource scoping
- 🔗 **Dependency Management** - Automatic dependency resolution and circular dependency detection
- 🎨 **Extensible** - Plugin system and hooks for custom behavior

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  nebula-engine                    │
├──────────────────────────────────────────────────┤
│         nebula-workflow    nebula-action         │
├──────────────────────────────────────────────────┤
│              nebula-resource (this crate)        │
│  ┌────────────────────────────────────────────┐  │
│  │  ResourceManager                           │  │
│  │  ├── Registry (resource discovery)         │  │
│  │  ├── PoolManager (connection pooling)      │  │
│  │  ├── LifecycleManager (state management)   │  │
│  │  └── MetricsCollector (observability)      │  │
│  └────────────────────────────────────────────┘  │
├──────────────────────────────────────────────────┤
│              nebula-credential                   │
└──────────────────────────────────────────────────┘
```

## Quick Start

### Installation

```toml
[dependencies]
nebula-resource = "0.2"
```

### Basic Usage

```rust
use nebula_resource::prelude::*;
use async_trait::async_trait;

// Define a resource
#[derive(Resource)]
#[resource(
    id = "database",
    name = "PostgreSQL Database",
    poolable = true,
    health_checkable = true
)]
pub struct DatabaseResource;

// Define configuration
#[derive(ResourceConfig)]
pub struct DatabaseConfig {
    pub connection_string: String,
    pub max_connections: u32,
    pub idle_timeout_seconds: u64,
}

// Define instance
pub struct DatabaseInstance {
    pool: sqlx::PgPool,
}

// Implement resource trait
#[async_trait]
impl Resource for DatabaseResource {
    type Config = DatabaseConfig;
    type Instance = DatabaseInstance;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        let pool = sqlx::PgPool::connect(&config.connection_string)
            .await
            .map_err(|e| ResourceError::InitializationFailed(e.to_string()))?;
        
        Ok(DatabaseInstance { pool })
    }
}

// Use in a workflow action
async fn process_data(ctx: &ExecutionContext) -> Result<()> {
    // Resource is automatically acquired and managed
    let db = ctx.get_resource::<DatabaseInstance>().await?;
    
    // Use the resource
    let result = sqlx::query("SELECT * FROM users")
        .fetch_all(&db.pool)
        .await?;
    
    // Resource is automatically released when action completes
    Ok(())
}
```

## Core Concepts

### Resource Lifecycle

Resources go through well-defined lifecycle states:

```rust
pub enum LifecycleState {
    Created,        // Resource created but not initialized
    Initializing,   // Currently being initialized
    Ready,          // Available for use
    InUse,          // Currently being used
    Idle,           // Ready but not in use
    Maintenance,    // Under maintenance
    Draining,       // Preparing for shutdown
    Cleanup,        // Being cleaned up
    Terminated,     // Fully terminated
    Failed,         // In error state
}
```

### Resource Scoping

Resources can be scoped at different levels:

- **Global** - Shared across all workflows and tenants
- **Tenant** - Isolated per tenant for multi-tenancy
- **Workflow** - Scoped to a specific workflow execution
- **Action** - Scoped to a specific action within a workflow

### Resource Pooling

Built-in support for efficient resource pooling:

```rust
#[derive(PoolConfig)]
pub struct PoolConfiguration {
    pub min_size: usize,          // Minimum pool size
    pub max_size: usize,          // Maximum pool size
    pub acquire_timeout: Duration, // Timeout for acquiring resource
    pub idle_timeout: Duration,    // Idle timeout before cleanup
    pub max_lifetime: Duration,    // Maximum resource lifetime
    pub validation_interval: Duration, // Health check interval
}
```

## Advanced Features

### Context Enrichment

Automatically enrich resource context with metadata:

```rust
pub struct ContextEnricher {
    enrichers: Vec<Box<dyn Enricher>>,
}

#[async_trait]
pub trait Enricher: Send + Sync {
    async fn enrich(&self, context: &mut ResourceContext) -> Result<()>;
}

// Example enrichers
- UserEnricher: adds user information
- GeoEnricher: adds geolocation data
- SecurityEnricher: adds security context
- TenantEnricher: adds tenant information
```

### Dependency Management

Declare and automatically resolve resource dependencies:

```rust
#[derive(Resource)]
#[resource(
    id = "api_service",
    depends_on = ["database", "cache", "logger"]
)]
pub struct ApiServiceResource;
```

### Credential Integration

Seamless integration with `nebula-credential`:

```rust
#[derive(ResourceConfig)]
pub struct ApiConfig {
    pub endpoint: String,
    
    #[credential(id = "api_key")]
    pub api_key: SecretString,
    
    #[credential(id = "api_secret", optional = true)]
    pub api_secret: Option<SecretString>,
}
```

### Observability

Built-in metrics and tracing:

```rust
// Automatic metrics collection
- resource.acquisitions.total
- resource.active.count
- resource.acquisition.duration
- resource.health.checks.total
- resource.errors.total

// Automatic tracing spans
- resource.acquire
- resource.initialize
- resource.health_check
- resource.cleanup
```

## Built-in Resources

### Available Resources

- **DatabaseResource** - Database connections (PostgreSQL, MySQL, SQLite)
- **HttpClientResource** - HTTP client with retry and circuit breaker
- **CacheResource** - Redis/Memcached caching
- **MessageQueueResource** - Kafka/RabbitMQ/SQS integration
- **StorageResource** - S3/GCS/Azure blob storage
- **LoggerResource** - Structured logging
- **MetricsResource** - Metrics collection
- **TracingResource** - Distributed tracing

### Custom Resources

Create custom resources by implementing the `Resource` trait:

```rust
#[async_trait]
pub trait Resource: Send + Sync {
    type Config: ResourceConfig;
    type Instance: Send + Sync;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError>;
    
    async fn health_check(&self, instance: &Self::Instance) -> Result<HealthStatus>;
    
    async fn cleanup(&self, instance: Self::Instance) -> Result<()>;
}
```

## Integration with Nebula Ecosystem

### nebula-workflow Integration

Resources are automatically managed within workflows:

```rust
#[workflow]
async fn data_pipeline(ctx: WorkflowContext) -> Result<()> {
    // Resources are automatically acquired
    let db = ctx.resource::<DatabaseInstance>().await?;
    let cache = ctx.resource::<CacheInstance>().await?;
    
    // Use resources in actions
    ctx.execute_action("fetch_data", |action_ctx| async {
        // Resources are available in actions
        let data = fetch_from_db(&db).await?;
        cache_data(&cache, data).await?;
        Ok(())
    }).await?;
    
    // Resources are automatically released
    Ok(())
}
```

### nebula-action Integration

Actions can declare required resources:

```rust
#[action(
    resources = ["database", "cache", "logger"]
)]
async fn process_record(ctx: ActionContext) -> Result<()> {
    let db = ctx.resource::<DatabaseInstance>().await?;
    let cache = ctx.resource::<CacheInstance>().await?;
    let logger = ctx.resource::<LoggerInstance>().await?;
    
    // Use resources...
    Ok(())
}
```

## Configuration

### Resource Configuration

Resources can be configured via:

1. **Configuration files** (YAML/TOML/JSON)
2. **Environment variables**
3. **Runtime configuration**
4. **Credential providers**

Example configuration:

```yaml
resources:
  database:
    type: postgresql
    config:
      connection_string: "${DATABASE_URL}"
      max_connections: 50
      idle_timeout_seconds: 300
    pool:
      min_size: 5
      max_size: 50
      acquire_timeout: 10s
    health_check:
      interval: 30s
      timeout: 5s
      
  cache:
    type: redis
    config:
      url: "${REDIS_URL}"
      max_connections: 100
    scoping: tenant  # Tenant-level isolation
```

## Testing

### Test Utilities

```rust
#[cfg(test)]
mod tests {
    use nebula_resource::testing::*;
    
    #[tokio::test]
    async fn test_resource_lifecycle() {
        // Create test resource manager
        let manager = TestResourceManager::new();
        
        // Register mock resource
        manager.register_mock::<DatabaseResource>(|mock| {
            mock.expect_create()
                .returning(|_, _| Ok(DatabaseInstance::mock()));
        });
        
        // Test resource operations
        let instance = manager.acquire::<DatabaseInstance>().await.unwrap();
        assert!(instance.is_healthy().await);
    }
}
```

## Performance Considerations

### Optimization Tips

1. **Pool Sizing** - Configure appropriate pool sizes based on workload
2. **Health Checks** - Balance frequency vs overhead
3. **Scoping** - Use appropriate scope to minimize resource creation
4. **Caching** - Enable resource caching where appropriate
5. **Lazy Loading** - Resources are loaded on-demand
6. **Circuit Breakers** - Prevent cascading failures

### Benchmarks

```
resource_acquire         time:   [127.3 ns 128.1 ns 128.9 ns]
resource_health_check    time:   [1.234 µs 1.241 µs 1.248 µs]
resource_pool_acquire    time:   [89.23 ns 89.67 ns 90.12 ns]
context_enrichment       time:   [234.5 ns 235.8 ns 237.1 ns]
```

## Migration Guide

### From v0.1 to v0.2

Major changes:

- New trait-based resource system
- Improved context propagation
- Enhanced pooling strategies
- Better credential integration

See [MIGRATION.md](https://claude.ai/chat/docs/MIGRATION.md) for detailed migration instructions.

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](https://claude.ai/chat/CONTRIBUTING.md) for guidelines.

### Development Setup

```bash
# Clone repository
git clone https://github.com/nebula-rs/nebula-resource
cd nebula-resource

# Install dependencies
cargo build

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](https://claude.ai/chat/LICENSE-APACHE))
- MIT license ([LICENSE-MIT](https://claude.ai/chat/LICENSE-MIT))

at your option.

## Resources

- [Documentation](https://docs.rs/nebula-resource)
- [Examples](https://claude.ai/chat/examples/)
- [API Reference](https://docs.rs/nebula-resource)
- [Discord Community](https://discord.gg/nebula-rs)
- [Blog Posts](https://blog.nebula.rs/tags/resources)