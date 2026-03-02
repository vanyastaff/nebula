# Nebula Resource

Resource lifecycle management for the Nebula workflow engine.

## Overview

`nebula-resource` provides a robust system for managing the lifecycle of resources used in workflows. It handles:

- **Resource Pooling**: Efficient reuse of expensive resources (connections, clients, etc.).
- **Dependency Management**: Initialization ordering based on resource dependencies.
- **Health Monitoring**: Background health checks and auto-removal of unhealthy resources.
- **Scoping**: Resource visibility and isolation (Global, Tenant, Workflow, Execution).
- **Provider Abstraction**: Decoupled resource acquisition via `ResourceProvider` trait.

## Features

- **Generic Resource Pool**: `Pool<R>` supports any type implementing the `Resource` trait.
- **Dependency Graph**: Detects cycles and ensures correct initialization order.
- **Async Health Checks**: Configurable background monitoring.
- **Graceful Shutdown**: Ensures all resources are cleaned up properly.
- **Type-Safe References**: `ResourceRef` wraps `TypeId` for compile-time type safety.
- **Dual Acquisition API**: Type-safe (`resource<R>()`) and dynamic (`acquire(id)`) resource access.

## Usage

### Basic Resource Management

```rust
use nebula_resource::{Manager, Resource, PoolConfig, Context, ExecutionId, Scope, WorkflowId};

// 1. Define your resource
struct MyResource; // implements Resource trait...

// 2. Initialize manager
let manager = Manager::new();

// 3. Register resource
manager.register(
    MyResource,
    MyConfig::default(),
    PoolConfig::default()
).unwrap();

// 4. Acquire resource
let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
let guard = manager.acquire("my-resource", &ctx).await.unwrap();

// Use the resource...
// It automatically returns to the pool when dropped.
```

### Using ResourceProvider (Decoupled Access)

```rust
use nebula_resource::{ResourceProvider, ResourceRef, Resource};

// Type-safe acquisition
struct PostgresResource;
impl Resource for PostgresResource {
    type Instance = PgConnection;
    fn id() -> &'static str { "postgres" }
}

// Acquire by type (preferred)
let conn = provider.resource::<PostgresResource>(&ctx).await?;

// Or acquire by ID (dynamic)
let any = provider.acquire("postgres", &ctx).await?;
```

See [RESOURCE_REF.md](./RESOURCE_REF.md) for detailed documentation on `ResourceRef` and `ResourceProvider`.

## Architecture

- **Manager**: The central entry point. Manages multiple pools and the dependency graph.
- **Pool**: Handles the actual storage, creation, and recycling of resource instances.
- **ResourceMetadata**: Static descriptor (key, name, description, icon, tags) for UI and discovery — similar to `ActionMetadata` in nebula-action. Implement `Resource::metadata()` to provide it; default builds from `id()`.
- **HealthChecker**: Runs sidecar tasks to verify resource health.
- **ResourceProvider**: Trait for decoupled resource acquisition (supports type-safe and dynamic access).
- **ResourceRef**: Type-safe wrapper around `TypeId` for resource identification.
