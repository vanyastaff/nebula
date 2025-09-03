---
title: Traits
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---
	
# Resource Traits Reference

## Core Traits

### `Resource`

The fundamental trait that all resources must implement.

```rust
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    /// Unique identifier for the resource
    fn id(&self) -> ResourceId;
    
    /// Resource type name
    fn resource_type(&self) -> &str;
    
    /// Initialize the resource
    async fn initialize(&self) -> Result<()>;
    
    /// Cleanup the resource
    async fn cleanup(&self) -> Result<()>;
    
    /// Perform health check
    async fn health_check(&self) -> Result<HealthStatus>;
    
    /// Get resource metrics
    async fn metrics(&self) -> Option<ResourceMetrics>;
    
    /// Validate resource configuration
    fn validate(&self) -> Result<()>;
    
    /// Get resource metadata
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::default()
    }
    
    /// Get resource capabilities
    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities::default()
    }
}
```

### `CloneableResource`

For resources that can be cloned.

```rust
pub trait CloneableResource: Resource + Clone {
    /// Clone the resource with a new ID
    fn clone_with_id(&self, id: ResourceId) -> Self;
    
    /// Deep clone including internal state
    fn deep_clone(&self) -> Result<Self>;
}
```

### `ConfigurableResource`

For resources with runtime configuration.

```rust
#[async_trait]
pub trait ConfigurableResource: Resource {
    type Config: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    
    /// Get current configuration
    fn config(&self) -> &Self::Config;
    
    /// Update configuration
    async fn update_config(&mut self, config: Self::Config) -> Result<()>;
    
    /// Validate configuration
    fn validate_config(config: &Self::Config) -> Result<()>;
    
    /// Get default configuration
    fn default_config() -> Self::Config;
}
```

## Lifecycle Traits

### `StatefulResource`

For resources with explicit state management.

```rust
#[async_trait]
pub trait StatefulResource: Resource {
    type State: Clone + Send + Sync;
    
    /// Get current state
    async fn state(&self) -> Self::State;
    
    /// Set state
    async fn set_state(&self, state: Self::State) -> Result<()>;
    
    /// Get state version
    async fn state_version(&self) -> Version;
    
    /// Transition to new state
    async fn transition(&self, event: StateEvent) -> Result<Self::State>;
    
    /// Subscribe to state changes
    async fn subscribe(&self) -> StateSubscription<Self::State>;
}
```

### `RefreshableResource`

For resources that can be refreshed.

```rust
#[async_trait]
pub trait RefreshableResource: Resource {
    /// Refresh the resource
    async fn refresh(&self) -> Result<()>;
    
    /// Check if refresh is needed
    async fn needs_refresh(&self) -> bool;
    
    /// Get last refresh time
    fn last_refresh(&self) -> Option<Instant>;
    
    /// Set refresh interval
    fn set_refresh_interval(&mut self, interval: Duration);
}
```

### `RetryableResource`

For resources with retry logic.

```rust
#[async_trait]
pub trait RetryableResource: Resource {
    /// Get retry policy
    fn retry_policy(&self) -> &RetryPolicy;
    
    /// Execute with retry
    async fn execute_with_retry<F, T>(&self, operation: F) -> Result<T>
    where
        F: Fn() -> Future<Output = Result<T>> + Send,
        T: Send;
    
    /// Check if operation should be retried
    fn should_retry(&self, error: &Error, attempt: u32) -> bool;
    
    /// Calculate retry delay
    fn retry_delay(&self, attempt: u32) -> Duration;
}
```

## Pool Traits

### `PoolableResource`

For resources that can be pooled.

```rust
#[async_trait]
pub trait PoolableResource: Resource + Clone {
    /// Check if resource is still valid for pooling
    async fn is_valid(&self) -> bool;
    
    /// Reset resource state for reuse
    async fn reset(&self) -> Result<()>;
    
    /// Called when acquired from pool
    async fn on_acquire(&self) -> Result<()>;
    
    /// Called when released to pool
    async fn on_release(&self) -> Result<()>;
    
    /// Get time since last use
    fn idle_time(&self) -> Duration;
    
    /// Check if resource should be evicted
    fn should_evict(&self) -> bool {
        self.idle_time() > Duration::from_secs(300)
    }
}
```

### `ResourceFactory`

For creating pooled resources.

```rust
#[async_trait]
pub trait ResourceFactory<R: Resource>: Send + Sync {
    /// Create a new resource instance
    async fn create(&self) -> Result<R>;
    
    /// Validate a resource
    async fn validate(&self, resource: &R) -> Result<()>;
    
    /// Destroy a resource
    async fn destroy(&self, resource: R) -> Result<()>;
    
    /// Get factory configuration
    fn config(&self) -> &FactoryConfig;
}
```

## Connection Traits

### `ConnectableResource`

For resources that maintain connections.

```rust
#[async_trait]
pub trait ConnectableResource: Resource {
    /// Connect to the resource
    async fn connect(&self) -> Result<()>;
    
    /// Disconnect from the resource
    async fn disconnect(&self) -> Result<()>;
    
    /// Check connection status
    async fn is_connected(&self) -> bool;
    
    /// Reconnect to the resource
    async fn reconnect(&self) -> Result<()> {
        self.disconnect().await?;
        self.connect().await
    }
    
    /// Get connection info
    fn connection_info(&self) -> ConnectionInfo;
}
```

### `StreamingResource`

For resources that provide streaming data.

```rust
#[async_trait]
pub trait StreamingResource: Resource {
    type Item: Send;
    type Stream: Stream<Item = Result<Self::Item>> + Send;
    
    /// Create a stream
    async fn stream(&self) -> Result<Self::Stream>;
    
    /// Subscribe to updates
    async fn subscribe(&self) -> Result<Self::Stream>;
    
    /// Process stream items
    async fn process_stream<F>(&self, processor: F) -> Result<()>
    where
        F: Fn(Self::Item) -> Future<Output = Result<()>> + Send;
}
```

## Monitoring Traits

### `ObservableResource`

For resources with observability features.

```rust
#[async_trait]
pub trait ObservableResource: Resource {
    /// Get resource events
    async fn events(&self) -> Vec<ResourceEvent>;
    
    /// Subscribe to events
    async fn subscribe_events(&self) -> EventSubscription;
    
    /// Record custom metric
    async fn record_metric(&self, name: &str, value: MetricValue);
    
    /// Get resource traces
    async fn traces(&self) -> Vec<Trace>;
    
    /// Add span to current trace
    fn span(&self, name: &str) -> Span;
}
```

### `HealthCheckable`

Extended health check capabilities.

```rust
#[async_trait]
pub trait HealthCheckable: Resource {
    /// Perform detailed health check
    async fn detailed_health_check(&self) -> DetailedHealthStatus;
    
    /// Get health history
    async fn health_history(&self) -> Vec<HealthCheckResult>;
    
    /// Register health check callback
    fn on_health_change(&self, callback: HealthCallback);
    
    /// Get health check configuration
    fn health_config(&self) -> &HealthCheckConfig;
}

pub struct DetailedHealthStatus {
    pub status: HealthStatus,
    pub checks: HashMap<String, CheckResult>,
    pub metadata: HashMap<String, Value>,
    pub timestamp: Instant,
}
```

## Security Traits

### `SecureResource`

For resources with security features.

```rust
#[async_trait]
pub trait SecureResource: Resource {
    /// Authenticate access to resource
    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken>;
    
    /// Authorize action on resource
    async fn authorize(&self, token: &AuthToken, action: &Action) -> Result<bool>;
    
    /// Encrypt data
    async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>>;
    
    /// Decrypt data
    async fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>>;
    
    /// Audit resource access
    async fn audit(&self, event: AuditEvent) -> Result<()>;
}
```

### `CredentialProvider`

For resources that provide credentials.

```rust
#[async_trait]
pub trait CredentialProvider: Resource {
    /// Get credentials
    async fn get_credentials(&self, name: &str) -> Result<Credentials>;
    
    /// Store credentials
    async fn store_credentials(&self, name: &str, credentials: Credentials) -> Result<()>;
    
    /// Rotate credentials
    async fn rotate_credentials(&self, name: &str) -> Result<Credentials>;
    
    /// List available credentials
    async fn list_credentials(&self) -> Result<Vec<String>>;
    
    /// Delete credentials
    async fn delete_credentials(&self, name: &str) -> Result<()>;
}
```

## Data Traits

### `DataResource`

For resources that manage data.

```rust
#[async_trait]
pub trait DataResource: Resource {
    type Data: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    
    /// Read data
    async fn read(&self, key: &str) -> Result<Option<Self::Data>>;
    
    /// Write data
    async fn write(&self, key: &str, data: Self::Data) -> Result<()>;
    
    /// Delete data
    async fn delete(&self, key: &str) -> Result<()>;
    
    /// List keys
    async fn list(&self, prefix: Option<&str>) -> Result<Vec<String>>;
    
    /// Batch operations
    async fn batch<O: DataOperation>(&self, operations: Vec<O>) -> Result<Vec<OperationResult>>;
}
```

### `CacheResource`

For cache resources.

```rust
#[async_trait]
pub trait CacheResource: Resource {
    type Value: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    
    /// Get value from cache
    async fn get(&self, key: &str) -> Result<Option<Self::Value>>;
    
    /// Set value in cache
    async fn set(&self, key: &str, value: Self::Value, ttl: Option<Duration>) -> Result<()>;
    
    /// Delete from cache
    async fn delete(&self, key: &str) -> Result<()>;
    
    /// Clear entire cache
    async fn clear(&self) -> Result<()>;
    
    /// Get cache statistics
    async fn stats(&self) -> CacheStats;
}
```

## Migration Traits

### `MigratableResource`

For resources that support migration.

```rust
#[async_trait]
pub trait MigratableResource: Resource {
    type State: Serialize + for<'de> Deserialize<'de>;
    
    /// Get current version
    fn version(&self) -> Version;
    
    /// Export state for migration
    async fn export_state(&self) -> Result<Self::State>;
    
    /// Import state from migration
    async fn import_state(&self, state: Self::State, version: Version) -> Result<()>;
    
    /// Migrate to new version
    async fn migrate_to(&self, version: Version) -> Result<()>;
    
    /// Check if migration is needed
    fn needs_migration(&self, target_version: Version) -> bool;
}
```

## Composition Traits

### `CompositeResource`

For resources composed of other resources.

```rust
#[async_trait]
pub trait CompositeResource: Resource {
    /// Get child resources
    async fn children(&self) -> Vec<Arc<dyn Resource>>;
    
    /// Add child resource
    async fn add_child(&self, resource: Arc<dyn Resource>) -> Result<()>;
    
    /// Remove child resource
    async fn remove_child(&self, id: &ResourceId) -> Result<()>;
    
    /// Get child by ID
    async fn get_child(&self, id: &ResourceId) -> Option<Arc<dyn Resource>>;
    
    /// Propagate operation to children
    async fn propagate<F>(&self, operation: F) -> Result<()>
    where
        F: Fn(Arc<dyn Resource>) -> Future<Output = Result<()>> + Send;
}
```

## Extension Traits

### `ResourceExt`

Extension methods for all resources.

```rust
pub trait ResourceExt: Resource {
    /// Convert to Any for downcasting
    fn as_any(&self) -> &dyn Any;
    
    /// Try to downcast
    fn downcast_ref<T: Resource + 'static>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
    
    /// Get resource as JSON
    fn to_json(&self) -> Result<Value> {
        Ok(json!({
            "id": self.id(),
            "type": self.resource_type(),
            "metadata": self.metadata(),
            "capabilities": self.capabilities(),
        }))
    }
    
    /// With timeout
    async fn with_timeout<F, T>(&self, duration: Duration, f: F) -> Result<T>
    where
        F: Future<Output = Result<T>> + Send,
        T: Send,
    {
        tokio::time::timeout(duration, f)
            .await
            .map_err(|_| Error::OperationTimeout {
                operation: "resource_operation".to_string(),
                duration,
            })?
    }
}

// Blanket implementation
impl<R: Resource> ResourceExt for R {}
```

## Implementing Custom Traits

### Example: Custom Database Trait

```rust
#[async_trait]
pub trait DatabaseResource: Resource + ConnectableResource {
    /// Execute query
    async fn execute(&self, query: &str, params: &[Value]) -> Result<QueryResult>;
    
    /// Begin transaction
    async fn begin_transaction(&self) -> Result<Transaction>;
    
    /// Prepare statement
    async fn prepare(&self, query: &str) -> Result<PreparedStatement>;
    
    /// Get database schema
    async fn schema(&self) -> Result<Schema>;
}

// Implementation
pub struct MyDatabase {
    // fields
}

#[async_trait]
impl Resource for MyDatabase {
    // Required implementations
}

#[async_trait]
impl ConnectableResource for MyDatabase {
    // Connection implementations
}

#[async_trait]
impl DatabaseResource for MyDatabase {
    async fn execute(&self, query: &str, params: &[Value]) -> Result<QueryResult> {
        // Implementation
    }
    
    // Other methods
}
```

## Trait Bounds

### Common Trait Bound Patterns

```rust
// Basic resource bound
fn process_resource<R: Resource>(resource: &R) -> Result<()> {
    // Process any resource
}

// Multiple trait bounds
fn process_poolable<R>(resource: &R) -> Result<()>
where
    R: Resource + PoolableResource + Clone,
{
    // Process poolable resource
}

// With async trait
async fn process_connectable<R>(resource: &R) -> Result<()>
where
    R: Resource + ConnectableResource + Send + Sync,
{
    if !resource.is_connected().await {
        resource.connect().await?;
    }
    // Process connected resource
}

// Generic with associated types
async fn process_data<R>(resource: &R) -> Result<()>
where
    R: DataResource,
    R::Data: Debug + Clone,
{
    let data = resource.read("key").await?;
    println!("Data: {:?}", data);
}
```

## Best Practices

1. **Keep traits focused** - Single responsibility per trait
2. **Use async trait** - For async methods in traits
3. **Provide default implementations** - Where sensible
4. **Use associated types** - For type-level dependencies
5. **Document trait requirements** - Clear contracts
6. **Test trait implementations** - Generic test suites
7. **Version traits carefully** - Breaking changes impact all implementors
8. **Use extension traits** - For optional functionality
9. **Compose traits** - Build complex resources from simple traits
10. **Consider object safety** - For dynamic dispatch needs