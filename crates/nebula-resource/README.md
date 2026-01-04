# nebula-resource

Comprehensive resource management framework for the Nebula workflow engine.

## Overview

`nebula-resource` provides lifecycle management, pooling, scoping, and observability for all resources used within workflows and actions (databases, HTTP clients, message queues, etc.).

## Key Features

- **Lifecycle Management** - Automatic initialization, health checks, and cleanup
- **Resource Pooling** - Efficient connection pooling with configurable strategies
- **Context Awareness** - Automatic context propagation for tracing and multi-tenancy
- **Credential Integration** - Seamless integration with `nebula-credential`
- **Built-in Observability** - Metrics, logging, and distributed tracing
- **Scoped Resources** - Global, Tenant, Workflow, and Action-level scoping
- **Dependency Management** - Automatic dependency resolution
- **Extensible** - Plugin system and hooks for custom behavior

## Quick Start

```rust
use nebula_resource::prelude::*;
use async_trait::async_trait;

// Define a resource
#[derive(Debug)]
pub struct DatabaseResource;

// Configuration
pub struct DatabaseConfig {
    pub connection_string: String,
    pub max_connections: u32,
    pub idle_timeout_seconds: u64,
}

// Instance
pub struct DatabaseInstance {
    // Your database connection
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
    ) -> ResourceResult<Self::Instance> {
        // Create and return instance
        todo!()
    }
}
```

## Resource Manager

```rust
use nebula_resource::{ResourceManager, ResourceManagerBuilder};

#[tokio::main]
async fn main() -> ResourceResult<()> {
    // Create manager
    let manager = ResourceManagerBuilder::new()
        .with_scope(ResourceScope::Global)
        .with_health_checks(true)
        .build()
        .await?;

    // Register resource
    manager.register("database", DatabaseResource).await?;

    // Get resource instance
    let db = manager.get::<DatabaseInstance>("database").await?;

    Ok(())
}
```

## Resource Pooling

```rust
use nebula_resource::pool::{PoolConfig, PoolStrategy};

let pool_config = PoolConfig {
    min_size: 5,
    max_size: 20,
    idle_timeout: Duration::from_secs(300),
    strategy: PoolStrategy::Fifo,
};

// Resource will be automatically pooled
manager.register_with_pool("database", DatabaseResource, pool_config).await?;
```

## Resource Scopes

```rust
use nebula_resource::ResourceScope;

// Global resources (shared across all tenants)
let global_scope = ResourceScope::Global;

// Tenant-specific resources
let tenant_scope = ResourceScope::Tenant(tenant_id);

// Workflow-specific resources
let workflow_scope = ResourceScope::Workflow(workflow_id);

// Execution-specific resources
let execution_scope = ResourceScope::Execution(execution_id);
```

## Health Checks

```rust
#[async_trait]
impl HealthCheckable for DatabaseInstance {
    async fn health_check(&self) -> ResourceResult<()> {
        // Ping database
        self.connection.ping().await?;
        Ok(())
    }
}

// Manager will automatically run health checks
manager.health_check("database").await?;
```

## Built-in Resources

```rust
// PostgreSQL
use nebula_resource::resources::postgres::PostgresResource;

// HTTP Client
use nebula_resource::resources::http::HttpClientResource;

// Redis
use nebula_resource::resources::redis::RedisResource;

// Kafka Producer
use nebula_resource::resources::kafka::KafkaProducerResource;
```

## Feature Flags

### Core Features
- `std` (default) - Standard library support
- `tokio` (default) - Tokio runtime
- `serde` (default) - Serialization support

### Database Support
- `postgres` - PostgreSQL driver
- `mysql` - MySQL driver
- `mongodb` - MongoDB driver
- `redis` - Redis driver

### Other Features
- `http-client` - HTTP client resource
- `kafka` - Kafka message queue
- `credentials` - Credential management integration
- `pooling` - Resource pooling
- `metrics` - Metrics collection
- `tracing` - Distributed tracing
- `testing` - Testing utilities

### Preset
- `full` - All features

## Context Propagation

```rust
use nebula_resource::ResourceContext;

let context = ResourceContext::new()
    .with_tenant_id(tenant_id)
    .with_user_id(user_id)
    .with_workflow_id(workflow_id)
    .with_execution_id(execution_id);

// Context is automatically propagated to all resources
let db = manager.get_with_context::<DatabaseInstance>("database", &context).await?;
```

## Error Handling

```rust
use nebula_resource::{ResourceError, ResourceResult};

async fn use_resource() -> ResourceResult<String> {
    let db = manager.get::<DatabaseInstance>("database").await?;

    let result = db.query("SELECT * FROM users")
        .await
        .map_err(|e| ResourceError::OperationFailed {
            resource: "database".to_string(),
            operation: "query".to_string(),
            source: e.into(),
        })?;

    Ok(result)
}
```

## Observability

```rust
// Metrics are automatically collected
manager.metrics().await;

// Logs are automatically generated
// Traces are automatically propagated (with `tracing` feature)
```

## Testing

```rust
#[cfg(feature = "testing")]
use nebula_resource::testing::MockResource;

#[tokio::test]
async fn test_with_mock() {
    let mock = MockResource::new()
        .with_response("query", "result");

    manager.register_mock("database", mock).await?;

    // Test your code
}
```

## Architecture

```
nebula-resource/
├── core/              # Core traits and types
├── manager/           # Resource manager
├── pool/              # Connection pooling
├── health/            # Health checking
├── observability/     # Metrics and tracing
├── stateful/          # Stateful resources
├── credentials/       # Credential integration
└── resources/         # Built-in resources
    ├── postgres/
    ├── mysql/
    ├── redis/
    ├── mongodb/
    ├── http/
    └── kafka/
```

## Best Practices

1. **Use scopes appropriately** - Global for shared, Execution for isolated
2. **Enable pooling** - For frequently used resources
3. **Implement health checks** - Detect failures early
4. **Use context** - For tracing and multi-tenancy
5. **Handle errors** - Resources can fail, plan for it

## License

Licensed under the same terms as the Nebula project.
