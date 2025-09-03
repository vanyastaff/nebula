---
title: API Reference
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# API Reference

## Core Module

### `ResourceManager`

The main entry point for resource management.

```rust
pub struct ResourceManager {
    registry: Arc<RwLock<ResourceRegistry>>,
    pool_manager: Arc<PoolManager>,
    lifecycle_manager: Arc<LifecycleManager>,
    metrics_collector: Arc<MetricsCollector>,
}
```

#### Methods

##### `new`

```rust
pub fn new(config: ResourceManagerConfig) -> Self
```

Creates a new ResourceManager instance.

**Parameters:**

- `config`: Configuration for the resource manager

**Returns:** `ResourceManager`

**Example:**

```rust
let config = ResourceManagerConfig::default();
let manager = ResourceManager::new(config);
```

---

##### `register_resource`

```rust
pub async fn register_resource<R>(&self, resource: R) -> Result<ResourceHandle<R>>
where
    R: Resource + Send + Sync + 'static,
```

Registers a new resource with the manager.

**Parameters:**

- `resource`: The resource instance to register

**Returns:** `Result<ResourceHandle<R>>`

**Errors:**

- `ResourceAlreadyExists`: If a resource with the same ID already exists
- `RegistrationFailed`: If registration fails for other reasons

**Example:**

```rust
let db = DatabaseResource::new(config);
let handle = manager.register_resource(db).await?;
```

---

##### `get_resource`

```rust
pub async fn get_resource<R>(&self, id: &ResourceId) -> Result<Arc<R>>
where
    R: Resource + 'static,
```

Retrieves a resource by ID.

**Parameters:**

- `id`: The resource identifier

**Returns:** `Result<Arc<R>>`

**Errors:**

- `ResourceNotFound`: If the resource doesn't exist
- `TypeMismatch`: If the resource exists but is of a different type

---

##### `remove_resource`

```rust
pub async fn remove_resource(&self, id: &ResourceId) -> Result<()>
```

Removes a resource from the manager.

**Parameters:**

- `id`: The resource identifier to remove

**Returns:** `Result<()>`

**Errors:**

- `ResourceNotFound`: If the resource doesn't exist
- `ResourceInUse`: If the resource is currently being used

---

### `Resource` Trait

The core trait that all resources must implement.

```rust
#[async_trait]
pub trait Resource: Send + Sync {
    /// Unique identifier for the resource
    fn id(&self) -> ResourceId;
    
    /// Resource type name
    fn resource_type(&self) -> &str;
    
    /// Initialize the resource
    async fn initialize(&self) -> Result<()>;
    
    /// Cleanup the resource
    async fn cleanup(&self) -> Result<()>;
    
    /// Health check
    async fn health_check(&self) -> Result<HealthStatus>;
    
    /// Get resource metrics
    async fn metrics(&self) -> Option<ResourceMetrics>;
    
    /// Validate resource configuration
    fn validate(&self) -> Result<()>;
}
```

#### Required Methods

##### `id`

Returns the unique identifier for the resource.

##### `resource_type`

Returns the type name of the resource (e.g., "database", "cache", "http_client").

##### `initialize`

Performs resource initialization. Called when the resource is first registered.

##### `cleanup`

Performs cleanup operations. Called when the resource is being removed.

##### `health_check`

Performs a health check and returns the current status.

##### `metrics`

Returns current resource metrics (optional).

##### `validate`

Validates the resource configuration.

---

### `ResourceHandle<R>`

A handle to a registered resource.

```rust
pub struct ResourceHandle<R: Resource> {
    inner: Arc<R>,
    id: ResourceId,
    manager: Weak<ResourceManager>,
}
```

#### Methods

##### `get`

```rust
pub fn get(&self) -> &R
```

Returns a reference to the resource.

##### `clone_inner`

```rust
pub fn clone_inner(&self) -> Arc<R>
```

Returns a cloned Arc to the resource.

##### `is_healthy`

```rust
pub async fn is_healthy(&self) -> bool
```

Checks if the resource is healthy.

##### `refresh`

```rust
pub async fn refresh(&self) -> Result<()>
```

Refreshes the resource (re-initializes if needed).

---

## Pool Module

### `PoolManager`

Manages pooled resources.

```rust
pub struct PoolManager {
    pools: Arc<RwLock<HashMap<ResourceId, Box<dyn ResourcePool>>>>,
    config: PoolConfig,
}
```

#### Methods

##### `create_pool`

```rust
pub async fn create_pool<R, F>(
    &self,
    id: ResourceId,
    factory: F,
    config: PoolConfig,
) -> Result<Pool<R>>
where
    R: Resource + Clone + 'static,
    F: ResourceFactory<R> + 'static,
```

Creates a new resource pool.

**Parameters:**

- `id`: Pool identifier
- `factory`: Factory for creating resources
- `config`: Pool configuration

**Returns:** `Result<Pool<R>>`

---

##### `get_pool`

```rust
pub async fn get_pool<R>(&self, id: &ResourceId) -> Result<Pool<R>>
where
    R: Resource + 'static,
```

Retrieves an existing pool.

---

### `Pool<R>`

A pool of resources.

```rust
pub struct Pool<R: Resource> {
    resources: Arc<RwLock<Vec<PooledResource<R>>>>,
    factory: Arc<dyn ResourceFactory<R>>,
    config: PoolConfig,
    metrics: Arc<PoolMetrics>,
}
```

#### Methods

##### `acquire`

```rust
pub async fn acquire(&self) -> Result<PooledHandle<R>>
```

Acquires a resource from the pool.

**Returns:** `Result<PooledHandle<R>>`

**Errors:**

- `PoolExhausted`: If no resources are available and max size is reached
- `AcquisitionTimeout`: If acquisition times out

##### `acquire_with_timeout`

```rust
pub async fn acquire_with_timeout(&self, timeout: Duration) -> Result<PooledHandle<R>>
```

Acquires a resource with a custom timeout.

##### `release`

```rust
pub async fn release(&self, handle: PooledHandle<R>)
```

Releases a resource back to the pool.

##### `size`

```rust
pub async fn size(&self) -> PoolSize
```

Returns current pool size information.

```rust
pub struct PoolSize {
    pub total: usize,
    pub available: usize,
    pub in_use: usize,
}
```

---

## Lifecycle Module

### `LifecycleManager`

Manages resource lifecycle events.

```rust
pub struct LifecycleManager {
    hooks: Arc<RwLock<Vec<Box<dyn LifecycleHook>>>>,
    states: Arc<RwLock<HashMap<ResourceId, LifecycleState>>>,
}
```

#### Methods

##### `register_hook`

```rust
pub async fn register_hook<H>(&self, hook: H)
where
    H: LifecycleHook + 'static,
```

Registers a lifecycle hook.

##### `transition`

```rust
pub async fn transition(
    &self,
    resource_id: &ResourceId,
    event: LifecycleEvent,
) -> Result<()>
```

Transitions a resource to a new lifecycle state.

---

### `LifecycleState`

Resource lifecycle states.

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleState {
    Created,
    Initializing,
    Ready,
    InUse,
    Idle,
    Refreshing,
    Draining,
    Cleanup,
    Terminated,
    Failed(String),
}
```

### `LifecycleEvent`

Lifecycle transition events.

```rust
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    Initialize,
    Ready,
    Acquire,
    Release,
    Refresh,
    Drain,
    Cleanup,
    Fail(String),
}
```

---

## Health Module

### `HealthChecker`

Performs health checks on resources.

```rust
pub struct HealthChecker {
    checks: Arc<RwLock<HashMap<ResourceId, HealthCheck>>>,
    config: HealthCheckConfig,
}
```

#### Methods

##### `check_health`

```rust
pub async fn check_health(&self, resource_id: &ResourceId) -> Result<HealthStatus>
```

Performs a health check on a specific resource.

##### `check_all`

```rust
pub async fn check_all(&self) -> HashMap<ResourceId, HealthStatus>
```

Performs health checks on all registered resources.

##### `start_monitoring`

```rust
pub async fn start_monitoring(&self, interval: Duration)
```

Starts continuous health monitoring.

---

### `HealthStatus`

Health status information.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy {
        message: Option<String>,
        latency: Duration,
    },
    Degraded {
        reason: String,
        latency: Duration,
    },
    Unhealthy {
        error: String,
        since: Instant,
    },
    Unknown,
}
```

---

## Metrics Module

### `MetricsCollector`

Collects and aggregates resource metrics.

```rust
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<ResourceId, ResourceMetrics>>>,
    aggregator: Arc<dyn MetricsAggregator>,
}
```

#### Methods

##### `record`

```rust
pub async fn record(&self, resource_id: &ResourceId, metric: Metric)
```

Records a metric for a resource.

##### `get_metrics`

```rust
pub async fn get_metrics(&self, resource_id: &ResourceId) -> Option<ResourceMetrics>
```

Gets metrics for a specific resource.

##### `get_aggregated`

```rust
pub async fn get_aggregated(&self) -> AggregatedMetrics
```

Gets aggregated metrics for all resources.

---

### `ResourceMetrics`

Resource metrics data.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub resource_id: ResourceId,
    pub resource_type: String,
    pub created_at: Instant,
    pub last_used: Instant,
    pub usage_count: u64,
    pub error_count: u64,
    pub success_rate: f64,
    pub avg_latency: Duration,
    pub p99_latency: Duration,
    pub memory_usage: usize,
    pub active_operations: usize,
    pub custom_metrics: HashMap<String, MetricValue>,
}
```

---

## Error Types

### `Error`

Main error type for the resource system.

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Resource not found: {0}")]
    ResourceNotFound(ResourceId),
    
    #[error("Resource already exists: {0}")]
    ResourceAlreadyExists(ResourceId),
    
    #[error("Resource initialization failed: {0}")]
    InitializationFailed(String),
    
    #[error("Health check failed: {0}")]
    HealthCheckFailed(String),
    
    #[error("Pool exhausted")]
    PoolExhausted,
    
    #[error("Acquisition timeout")]
    AcquisitionTimeout,
    
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: String,
        actual: String,
    },
    
    #[error("Resource in use")]
    ResourceInUse,
    
    #[error("Migration failed: {0}")]
    MigrationFailed(String),
    
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Other error: {0}")]
    Other(String),
}
```

---

## Builder APIs

### `ResourceBuilder`

Fluent builder for creating resources.

```rust
pub struct ResourceBuilder<R> {
    config: HashMap<String, Value>,
    phantom: PhantomData<R>,
}
```

#### Methods

##### `new`

```rust
pub fn new() -> Self
```

Creates a new builder.

##### `with`

```rust
pub fn with<K, V>(mut self, key: K, value: V) -> Self
where
    K: Into<String>,
    V: Serialize,
```

Adds a configuration value.

##### `build`

```rust
pub fn build(self) -> Result<R>
where
    R: Resource + Default,
```

Builds the resource.

**Example:**

```rust
let resource = ResourceBuilder::<DatabaseResource>::new()
    .with("host", "localhost")
    .with("port", 5432)
    .with("database", "mydb")
    .build()?;
```

---

## Async APIs

All async methods return `Future`s that are `Send + Sync + 'static`.

### Cancellation Safety

Most async operations are cancellation-safe:

- `acquire`: Safe - no resource is leaked
- `release`: Safe - resource is properly returned
- `initialize`: **Not safe** - may leave resource partially initialized
- `cleanup`: **Not safe** - may leave resource partially cleaned up

### Example with Timeout

```rust
use tokio::time::timeout;
use std::time::Duration;

// Safe cancellation with timeout
let result = timeout(
    Duration::from_secs(5),
    pool.acquire()
).await;

match result {
    Ok(Ok(handle)) => {
        // Use resource
        handle.do_work().await?;
    }
    Ok(Err(e)) => {
        // Handle pool error
        eprintln!("Pool error: {}", e);
    }
    Err(_) => {
        // Handle timeout
        eprintln!("Acquisition timed out");
    }
}
```

---

## Extension Traits

### `ResourceExt`

Extension trait for additional resource functionality.

```rust
pub trait ResourceExt: Resource {
    /// Get resource as Any for downcasting
    fn as_any(&self) -> &dyn Any;
    
    /// Try to downcast to a specific type
    fn downcast_ref<T: Resource + 'static>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
    
    /// Clone the resource if it implements Clone
    fn try_clone(&self) -> Option<Box<dyn Resource>>;
}
```

### `PoolExt`

Extension trait for pool operations.

```rust
pub trait PoolExt<R: Resource> {
    /// Acquire multiple resources
    async fn acquire_many(&self, count: usize) -> Result<Vec<PooledHandle<R>>>;
    
    /// Resize the pool
    async fn resize(&self, new_size: usize) -> Result<()>;
    
    /// Drain all resources
    async fn drain(&self) -> Result<Vec<R>>;
}
```

---

## Macros

### `resource!`

Macro for implementing the Resource trait.

```rust
resource! {
    name: MyResource,
    type: "my_resource",
    fields: {
        config: MyConfig,
        state: Arc<RwLock<State>>,
    }
}
```

Expands to a complete Resource implementation.

### `pool!`

Macro for creating resource pools.

```rust
let pool = pool! {
    resource: DatabaseConnection,
    size: 10,
    timeout: 30s,
    factory: || DatabaseConnection::new(config.clone()),
};
```

---

## Global Functions

### `init_resource_system`

```rust
pub fn init_resource_system(config: SystemConfig) -> Result<()>
```

Initializes the global resource system.

### `get_global_manager`

```rust
pub fn get_global_manager() -> Arc<ResourceManager>
```

Gets the global resource manager instance.

### `shutdown_resource_system`

```rust
pub async fn shutdown_resource_system() -> Result<()>
```

Shuts down the resource system, cleaning up all resources.