//! Resource manager implementation

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::{LifecycleEvent, LifecycleState},
    resource::{
        Resource, ResourceFactory, ResourceGuard, ResourceId, ResourceInstance,
        ResourceInstanceMetadata, ResourceMetadata, TypedResourceInstance,
    },
    scoping::{ResourceScope, ScopingStrategy},
    traits::{HealthCheckable, HealthStatus, Poolable},
};

#[cfg(feature = "pooling")]
use crate::pool::{PoolManager, ResourcePool};

/// Central manager for all resource operations
pub struct ResourceManager {
    /// Registry of resource factories
    registry: Arc<RwLock<HashMap<ResourceId, Arc<dyn ResourceFactory>>>>,

    /// Active resource instances by scope and type
    instances: Arc<DashMap<String, Arc<dyn Any + Send + Sync>>>,

    /// Resource metadata cache
    metadata_cache: Arc<DashMap<ResourceId, ResourceMetadata>>,

    /// Lifecycle event subscribers
    event_subscribers: Arc<RwLock<Vec<futures::channel::mpsc::UnboundedSender<LifecycleEvent>>>>,

    /// Pool manager for resource pooling
    #[cfg(feature = "pooling")]
    pool_manager: Arc<PoolManager>,

    /// Configuration
    config: ResourceManagerConfig,

    /// Scoping strategy
    scoping_strategy: ScopingStrategy,
}

/// Configuration for the resource manager
#[derive(Debug, Clone)]
pub struct ResourceManagerConfig {
    /// Default timeout for resource operations
    pub default_timeout: Duration,
    /// Maximum number of instances per resource type
    pub max_instances_per_type: usize,
    /// Health check interval for active resources
    pub health_check_interval: Duration,
    /// Whether to enable automatic cleanup of idle resources
    pub auto_cleanup_enabled: bool,
    /// Idle timeout before automatic cleanup
    pub idle_cleanup_timeout: Duration,
}

impl Default for ResourceManagerConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            max_instances_per_type: 100,
            health_check_interval: Duration::from_secs(60),
            auto_cleanup_enabled: true,
            idle_cleanup_timeout: Duration::from_secs(300),
        }
    }
}

impl ResourceManager {
    /// Create a new resource manager with default configuration
    pub fn new() -> Self {
        Self::with_config(ResourceManagerConfig::default())
    }

    /// Create a new resource manager with custom configuration
    pub fn with_config(config: ResourceManagerConfig) -> Self {
        Self {
            registry: Arc::new(RwLock::new(HashMap::new())),
            instances: Arc::new(DashMap::new()),
            metadata_cache: Arc::new(DashMap::new()),
            event_subscribers: Arc::new(RwLock::new(Vec::new())),
            #[cfg(feature = "pooling")]
            pool_manager: Arc::new(PoolManager::new()),
            config,
            scoping_strategy: ScopingStrategy::default(),
        }
    }

    /// Create a builder for more advanced configuration
    pub fn builder() -> ResourceManagerBuilder {
        ResourceManagerBuilder::new()
    }

    /// Register a resource type with the manager
    pub fn register<R>(&self, resource: R) -> ResourceResult<()>
    where
        R: Resource + 'static,
    {
        let metadata = resource.metadata();
        let resource_id = metadata.id.clone();

        // Store metadata in cache
        self.metadata_cache.insert(resource_id.clone(), metadata);

        // Create factory wrapper
        let factory = Arc::new(ResourceFactoryWrapper::new(resource));

        // Register in registry
        {
            let mut registry = self.registry.write();
            registry.insert(resource_id.clone(), factory);
        }

        // Emit registration event
        self.emit_lifecycle_event(LifecycleEvent::new(
            resource_id.unique_key(),
            LifecycleState::Created,
            LifecycleState::Ready,
        ));

        Ok(())
    }

    /// Get a resource instance, creating it if necessary
    pub async fn get<T>(&self, context: &ResourceContext) -> ResourceResult<ResourceGuard<T>>
    where
        T: Send + Sync + 'static,
    {
        let resource_id = self.find_resource_id_for_type::<T>()?;
        self.get_by_id(&resource_id, context).await
    }

    /// Get a resource instance by resource ID
    pub async fn get_by_id<T>(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<T>>
    where
        T: Send + Sync + 'static,
    {
        let instance_key = self.build_instance_key(resource_id, &context.scope);

        // Try to get existing instance
        if let Some(instance) = self.instances.get(&instance_key) {
            if let Ok(typed_instance) = self.cast_to_typed_instance::<T>(instance.value()) {
                return Ok(self.create_guard(typed_instance));
            }
        }

        // Create new instance
        self.create_instance(resource_id, context).await
    }

    /// Create a new resource instance
    async fn create_instance<T>(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<T>>
    where
        T: Send + Sync + 'static,
    {
        // Get factory
        let factory = {
            let registry = self.registry.read();
            registry
                .get(resource_id)
                .cloned()
                .ok_or_else(|| ResourceError::unavailable(
                    resource_id.unique_key(),
                    "Resource type not registered",
                    false,
                ))?
        };

        // Emit creation start event
        self.emit_lifecycle_event(LifecycleEvent::new(
            resource_id.unique_key(),
            LifecycleState::Created,
            LifecycleState::Initializing,
        ));

        // Create configuration (simplified for now)
        let config = serde_json::json!({});
        let dependencies = HashMap::new();

        // Create instance through factory
        let instance = factory
            .create_instance(config, context, &dependencies)
            .await?;

        // Cast to typed instance
        let typed_instance = self.cast_to_typed_instance::<T>(&instance)?;

        // Store instance
        let instance_key = self.build_instance_key(resource_id, &context.scope);
        self.instances.insert(instance_key, instance);

        // Emit ready event
        self.emit_lifecycle_event(LifecycleEvent::new(
            resource_id.unique_key(),
            LifecycleState::Initializing,
            LifecycleState::Ready,
        ));

        Ok(self.create_guard(typed_instance))
    }

    /// Subscribe to lifecycle events
    pub fn subscribe_to_events(&self) -> futures::channel::mpsc::UnboundedReceiver<LifecycleEvent> {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        {
            let mut subscribers = self.event_subscribers.write();
            subscribers.push(sender);
        }
        receiver
    }

    /// Get metadata for all registered resource types
    pub fn list_registered_types(&self) -> Vec<ResourceMetadata> {
        self.metadata_cache
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get metadata for a specific resource type
    pub fn get_metadata(&self, resource_id: &ResourceId) -> Option<ResourceMetadata> {
        self.metadata_cache.get(resource_id).map(|entry| entry.value().clone())
    }

    /// Shutdown the resource manager and cleanup all resources
    pub async fn shutdown(&self) -> ResourceResult<()> {
        // TODO: Implement graceful shutdown
        // - Stop accepting new requests
        // - Wait for active operations to complete
        // - Cleanup all resources
        // - Close pools
        Ok(())
    }

    // Helper methods

    fn find_resource_id_for_type<T>(&self) -> ResourceResult<ResourceId>
    where
        T: 'static,
    {
        let type_id = TypeId::of::<T>();

        // For now, we'll use a simple approach
        // In a real implementation, we'd maintain a TypeId -> ResourceId mapping
        for entry in self.metadata_cache.iter() {
            // This is a simplified check - in reality we'd need better type mapping
            if entry.key().name.contains(&std::any::type_name::<T>()) {
                return Ok(entry.key().clone());
            }
        }

        Err(ResourceError::unavailable(
            "unknown",
            format!("No resource registered for type {}", std::any::type_name::<T>()),
            false,
        ))
    }

    fn build_instance_key(&self, resource_id: &ResourceId, scope: &ResourceScope) -> String {
        format!("{}:{}", resource_id.unique_key(), scope.scope_key())
    }

    fn cast_to_typed_instance<T>(
        &self,
        instance: &Arc<dyn Any + Send + Sync>,
    ) -> ResourceResult<TypedResourceInstance<T>>
    where
        T: Send + Sync + 'static,
    {
        // This is a simplified cast - in reality we'd need more sophisticated type handling
        instance
            .downcast_ref::<TypedResourceInstance<T>>()
            .ok_or_else(|| ResourceError::internal(
                "unknown",
                "Failed to cast instance to requested type",
            ))
            .map(|typed| typed.clone())
    }

    fn create_guard<T>(&self, instance: TypedResourceInstance<T>) -> ResourceGuard<T>
    where
        T: Send + Sync + 'static,
    {
        let instance_id = instance.instance_id();
        let resource_id = instance.resource_id().clone();

        ResourceGuard::new(instance, move |_instance| {
            // Cleanup callback when guard is dropped
            // In a real implementation, this would handle returning to pool, cleanup, etc.
        })
    }

    fn emit_lifecycle_event(&self, event: LifecycleEvent) {
        let subscribers = self.event_subscribers.read();
        for sender in subscribers.iter() {
            let _ = sender.unbounded_send(event.clone());
        }
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for ResourceManager with advanced configuration options
pub struct ResourceManagerBuilder {
    config: ResourceManagerConfig,
    scoping_strategy: ScopingStrategy,
}

impl ResourceManagerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: ResourceManagerConfig::default(),
            scoping_strategy: ScopingStrategy::default(),
        }
    }

    /// Set the default timeout
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.config.default_timeout = timeout;
        self
    }

    /// Set the maximum instances per type
    pub fn max_instances_per_type(mut self, max: usize) -> Self {
        self.config.max_instances_per_type = max;
        self
    }

    /// Set the health check interval
    pub fn health_check_interval(mut self, interval: Duration) -> Self {
        self.config.health_check_interval = interval;
        self
    }

    /// Enable or disable automatic cleanup
    pub fn auto_cleanup(mut self, enabled: bool) -> Self {
        self.config.auto_cleanup_enabled = enabled;
        self
    }

    /// Set the scoping strategy
    pub fn scoping_strategy(mut self, strategy: ScopingStrategy) -> Self {
        self.scoping_strategy = strategy;
        self
    }

    /// Build the resource manager
    pub fn build(self) -> ResourceManager {
        let mut manager = ResourceManager::with_config(self.config);
        manager.scoping_strategy = self.scoping_strategy;
        manager
    }
}

impl Default for ResourceManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper that adapts Resource trait to ResourceFactory trait
struct ResourceFactoryWrapper<R> {
    resource: R,
}

impl<R> ResourceFactoryWrapper<R>
where
    R: Resource,
{
    fn new(resource: R) -> Self {
        Self { resource }
    }
}

#[async_trait]
impl<R> ResourceFactory for ResourceFactoryWrapper<R>
where
    R: Resource + Send + Sync + 'static,
{
    async fn create_instance(
        &self,
        config: serde_json::Value,
        context: &ResourceContext,
        _dependencies: &HashMap<ResourceId, Arc<dyn Any + Send + Sync>>,
    ) -> ResourceResult<Arc<dyn Any + Send + Sync>> {
        // For now, create a default config
        // In a real implementation, we'd deserialize the JSON config
        let default_config = serde_json::from_value(config).map_err(|e| {
            ResourceError::configuration(format!("Failed to deserialize config: {}", e))
        })?;

        // Create the instance
        let instance = self.resource.create(&default_config, context).await?;

        // Wrap in TypedResourceInstance
        let metadata = ResourceInstanceMetadata {
            instance_id: Uuid::new_v4(),
            resource_id: self.resource.metadata().id.clone(),
            state: LifecycleState::Ready,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed_at: None,
            tags: std::collections::HashMap::new(),
        };

        let typed_instance = TypedResourceInstance::new(Arc::new(instance), metadata);
        Ok(Arc::new(typed_instance))
    }

    fn metadata(&self) -> ResourceMetadata {
        self.resource.metadata()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ResourceContext;

    // Mock resource for testing
    struct TestResource;

    struct TestConfig;
    impl ResourceConfig for TestConfig {
        fn merge(&mut self, _other: Self) {}
    }

    struct TestInstance {
        id: Uuid,
    }

    impl ResourceInstance for TestInstance {
        fn instance_id(&self) -> Uuid { self.id }
        fn resource_id(&self) -> &ResourceId { todo!() }
        fn lifecycle_state(&self) -> LifecycleState { LifecycleState::Ready }
        fn context(&self) -> &ResourceContext { todo!() }
        fn created_at(&self) -> chrono::DateTime<chrono::Utc> { chrono::Utc::now() }
        fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> { None }
        fn touch(&mut self) {}
    }

    #[async_trait]
    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = TestInstance;

        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::new(
                ResourceId::new("test", "1.0"),
                "Test resource".to_string(),
            )
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _context: &ResourceContext,
        ) -> ResourceResult<Self::Instance> {
            Ok(TestInstance {
                id: Uuid::new_v4(),
            })
        }
    }

    #[tokio::test]
    async fn test_resource_manager_creation() {
        let manager = ResourceManager::new();
        assert!(manager.list_registered_types().is_empty());
    }

    #[tokio::test]
    async fn test_resource_registration() {
        let manager = ResourceManager::new();
        let resource = TestResource;

        manager.register(resource).unwrap();

        let types = manager.list_registered_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].id.name, "test");
    }
}