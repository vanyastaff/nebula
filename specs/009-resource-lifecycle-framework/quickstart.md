# Quickstart: nebula-resource

**Branch**: `009-resource-lifecycle-framework`
**Date**: 2026-02-15

## Overview

`nebula-resource` manages the lifecycle of external service connections (databases, caches, HTTP clients, message queues) for the Nebula workflow engine. It handles pooling, health monitoring, dependency ordering, scope isolation, and graceful shutdown — so resource authors only need to implement a minimal contract.

## 1. Implement the Resource Trait

Define how to create, validate, recycle, and clean up your resource.

```rust
use nebula_resource::{Resource, Config, Context, Error};

/// Configuration for our cache resource.
pub struct CacheConfig {
    pub url: String,
    pub max_entries: usize,
}

impl Config for CacheConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.url.is_empty() {
            return Err(Error::configuration("url must not be empty"));
        }
        if self.max_entries == 0 {
            return Err(Error::configuration("max_entries must be > 0"));
        }
        Ok(())
    }
}

/// A cache client (the actual connection/instance).
pub struct CacheClient {
    // ... connection handle ...
}

/// The resource factory.
pub struct CacheResource;

impl Resource for CacheResource {
    type Config = CacheConfig;
    type Instance = CacheClient;

    fn id(&self) -> &str {
        "cache"
    }

    async fn create(
        &self,
        config: &CacheConfig,
        _context: &Context,
    ) -> Result<CacheClient, Error> {
        // Connect to the cache service
        let client = CacheClient { /* ... */ };
        Ok(client)
    }

    async fn is_valid(&self, _instance: &CacheClient) -> Result<bool, Error> {
        // Check if the connection is still alive (e.g., PING)
        Ok(true)
    }

    async fn cleanup(&self, _instance: CacheClient) -> Result<(), Error> {
        // Close the connection
        Ok(())
    }
}
```

## 2. Register with the Manager

```rust
use nebula_resource::{Manager, PoolConfig};
use std::sync::Arc;

let manager = Arc::new(Manager::new());

let config = CacheConfig {
    url: "redis://localhost:6379".into(),
    max_entries: 1000,
};

let pool_config = PoolConfig {
    min_size: 2,
    max_size: 10,
    ..PoolConfig::default()
};

manager.register(CacheResource, config, pool_config).await?;
```

## 3. Acquire from an Action

```rust
use nebula_action::prelude::*;

pub struct MyCacheAction;

impl ProcessAction for MyCacheAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Acquire the cache resource by key
        let resource = ctx.resource("cache").await?;

        // Downcast to the concrete handle type
        let handle = resource.downcast::<nebula_resource::ResourceHandle>()
            .map_err(|_| ActionError::fatal("unexpected resource type"))?;

        // Use the resource...

        // Guard is dropped here — instance returned to pool automatically
        ActionResult::success(serde_json::json!({ "status": "ok" }))
    }
}
```

## 4. Wire Up in the Engine

```rust
use nebula_engine::WorkflowEngine;

let engine = WorkflowEngine::new(runtime, event_bus, metrics)
    .with_resource_manager(manager);

// Resources are now available to all actions via ActionContext
```

## 5. Add Health Checking (Optional)

```rust
use nebula_resource::{HealthCheckable, HealthStatus, HealthState};
use std::time::Duration;

impl HealthCheckable for CacheResource {
    async fn health_check(&self) -> Result<HealthStatus, Error> {
        // PING the cache
        Ok(HealthStatus {
            state: HealthState::Healthy,
            latency: Some(Duration::from_millis(1)),
            metadata: Default::default(),
        })
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}
```

## 6. Resource Dependencies

If your resource depends on another (e.g., a service client that needs a database):

```rust
impl Resource for MyServiceResource {
    // ...

    fn dependencies(&self) -> Vec<&str> {
        vec!["postgres"]  // Must be registered and initialized first
    }
}
```

The manager initializes resources in topological order and rejects circular dependencies.

## Key Concepts

| Concept | Description |
|---------|-------------|
| **Resource** | Factory that creates/validates/recycles/cleans up instances |
| **Pool** | Manages reusable instances with size limits and timeouts |
| **Guard** | RAII wrapper — auto-returns instance to pool on drop |
| **Manager** | Central registry — register resources, acquire instances |
| **Scope** | Access control — global, tenant, workflow, execution, action |
| **HealthChecker** | Background monitor — periodic health checks per instance |
| **DependencyGraph** | Topological ordering of resource initialization |

## Configuration Defaults

| Setting | Default | Description |
|---------|---------|-------------|
| `min_size` | 1 | Minimum idle instances in pool |
| `max_size` | 10 | Maximum total instances in pool |
| `acquire_timeout` | 30s | Max wait time when pool is full |
| `idle_timeout` | 600s | Max idle time before instance is cleaned up |
| `max_lifetime` | 3600s | Max total lifetime of an instance |
| `health_check_interval` | 30s | Time between health checks |
| `health_check_timeout` | 5s | Max time for a single health check |
