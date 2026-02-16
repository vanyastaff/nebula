# Nebula Resource

Resource lifecycle management for the Nebula workflow engine.

## Overview

`nebula-resource` provides a robust system for managing the lifecycle of resources used in workflows. It handles:

- **Resource Pooling**: Efficient reuse of expensive resources (connections, clients, etc.).
- **Dependency Management**: Initialization ordering based on resource dependencies.
- **Health Monitoring**: Background health checks and auto-removal of unhealthy resources.
- **Scoping**: Resource visibility and isolation (Global, Tenant, Workflow, Execution).

## Features

- **Generic Resource Pool**: `Pool<R>` supports any type implementing the `Resource` trait.
- **Dependency Graph**: Detects cycles and ensures correct initialization order.
- **Async Health Checks**: Configurable background monitoring.
- **Graceful Shutdown**: Ensures all resources are cleaned up properly.

## Usage

```rust
use nebula_resource::{Manager, Resource, PoolConfig, Context, Scope};

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
let ctx = Context::new(Scope::Global, "workflow-id", "execution-id");
let guard = manager.acquire("my-resource", &ctx).await.unwrap();

// Use the resource...
// It automatically returns to the pool when dropped.
```

## Architecture

- **Manager**: The central entry point. Manages multiple pools and the dependency graph.
- **Pool**: Handles the actual storage, creation, and recycling of resource instances.
- **HealthChecker**: Runs sidecar tasks to verify resource health.
