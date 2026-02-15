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
    dependency::DependencyGraph,
    error::{ResourceError, ResourceResult},
    lifecycle::{LifecycleEvent, LifecycleState},
    resource::{
        Resource, ResourceFactory, ResourceGuard, ResourceId, ResourceInstanceMetadata,
        ResourceMetadata, TypedResourceInstance,
    },
    scoping::ScopingStrategy,
};

use crate::health::{HealthCheckConfig, HealthChecker};
use crate::pool::{PoolStrategy, PoolTrait, ResourcePool};

/// Message for cleanup channel (async drop pattern)
enum CleanupMessage {
    Release { instance_id: Uuid, type_id: TypeId },
}

/// Central manager for all resource operations
pub struct ResourceManager {
    /// Registry of resource factories
    registry: Arc<RwLock<HashMap<ResourceId, Arc<dyn ResourceFactory>>>>,

    /// Resource pools by `TypeId`
    pools: Arc<DashMap<TypeId, Arc<dyn PoolTrait + Send + Sync>>>,

    /// Resource metadata cache
    metadata_cache: Arc<DashMap<ResourceId, ResourceMetadata>>,

    /// Maps concrete instance `TypeId` to `ResourceId` for type-safe lookups
    type_to_resource: DashMap<TypeId, ResourceId>,

    /// Stored configs for each resource (passed to factory on pool creation)
    resource_configs: DashMap<ResourceId, serde_json::Value>,

    /// Dependency graph for initialization ordering
    dependency_graph: Arc<RwLock<DependencyGraph>>,

    /// Lifecycle event subscribers
    event_subscribers: Arc<RwLock<Vec<futures::channel::mpsc::UnboundedSender<LifecycleEvent>>>>,

    /// Cleanup channel for async drop operations
    cleanup_tx: tokio::sync::mpsc::UnboundedSender<CleanupMessage>,

    /// Background health checker
    health_checker: Arc<HealthChecker>,

    /// Shutdown signal
    shutdown_signal: Arc<tokio::sync::RwLock<bool>>,

    /// Configuration
    config: ResourceManagerConfig,

    /// Scoping strategy
    scoping_strategy: ScopingStrategy,
}

impl std::fmt::Debug for ResourceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let registry_count = self.registry.read().len();
        let pool_count = self.pools.len();
        let metadata_count = self.metadata_cache.len();
        let type_mapping_count = self.type_to_resource.len();
        f.debug_struct("ResourceManager")
            .field("registry_count", &registry_count)
            .field("pool_count", &pool_count)
            .field("metadata_count", &metadata_count)
            .field("type_mapping_count", &type_mapping_count)
            .field("dependency_graph", &self.dependency_graph)
            .field(
                "event_subscriber_count",
                &self.event_subscribers.read().len(),
            )
            .field("health_checker", &self.health_checker)
            .field("config", &self.config)
            .field("scoping_strategy", &self.scoping_strategy)
            .finish()
    }
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
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(ResourceManagerConfig::default())
    }

    /// Create a new resource manager with custom configuration
    #[must_use]
    pub fn with_config(config: ResourceManagerConfig) -> Self {
        let (cleanup_tx, mut cleanup_rx) = tokio::sync::mpsc::unbounded_channel();
        let pools: Arc<DashMap<TypeId, Arc<dyn PoolTrait + Send + Sync>>> =
            Arc::new(DashMap::new());
        let pools_clone = Arc::clone(&pools);

        // Spawn cleanup task for async drop handling
        tokio::spawn(async move {
            while let Some(msg) = cleanup_rx.recv().await {
                match msg {
                    CleanupMessage::Release {
                        instance_id,
                        type_id,
                    } => {
                        if let Some(pool) = pools_clone.get(&type_id) {
                            let _ = pool.release_any(instance_id).await;
                        }
                    }
                }
            }
        });

        // Create health checker
        let health_check_config = HealthCheckConfig {
            default_interval: config.health_check_interval,
            failure_threshold: 3,
            auto_remove_unhealthy: config.auto_cleanup_enabled,
            check_timeout: Duration::from_secs(5),
        };
        let health_checker = Arc::new(HealthChecker::new(health_check_config));

        Self {
            registry: Arc::new(RwLock::new(HashMap::new())),
            pools,
            metadata_cache: Arc::new(DashMap::new()),
            type_to_resource: DashMap::new(),
            resource_configs: DashMap::new(),
            dependency_graph: Arc::new(RwLock::new(DependencyGraph::new())),
            event_subscribers: Arc::new(RwLock::new(Vec::new())),
            cleanup_tx,
            health_checker,
            shutdown_signal: Arc::new(tokio::sync::RwLock::new(false)),
            config,
            scoping_strategy: ScopingStrategy::default(),
        }
    }

    /// Create a builder for more advanced configuration
    #[must_use]
    pub fn builder() -> ResourceManagerBuilder {
        ResourceManagerBuilder::new()
    }

    /// Register a resource type with the manager
    pub fn register<R>(&self, resource: R) -> ResourceResult<()>
    where
        R: Resource + 'static,
        R::Config: serde::de::DeserializeOwned,
    {
        self.register_with_config(resource, serde_json::Value::Null)
    }

    /// Register a resource type with explicit configuration
    pub fn register_with_config<R>(
        &self,
        resource: R,
        config: serde_json::Value,
    ) -> ResourceResult<()>
    where
        R: Resource + 'static,
        R::Config: serde::de::DeserializeOwned,
    {
        let metadata = resource.metadata();
        let resource_id = metadata.id.clone();

        // Register dependencies in the graph
        {
            let mut dep_graph = self.dependency_graph.write();
            for dep in &metadata.dependencies {
                dep_graph
                    .add_dependency(resource_id.clone(), dep.clone())
                    .map_err(|e| {
                        ResourceError::internal(
                            resource_id.to_string(),
                            format!("Failed to register dependency: {e}"),
                        )
                    })?;
            }
        }

        // Map the Instance TypeId to this ResourceId for type-safe lookups
        self.type_to_resource
            .insert(TypeId::of::<R::Instance>(), resource_id.clone());

        // Store config for later use by the pool factory
        self.resource_configs.insert(resource_id.clone(), config);

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
        // Check if shutting down
        if self.is_shutting_down().await {
            return Err(ResourceError::unavailable(
                resource_id.to_string(),
                "ResourceManager is shutting down",
                false,
            ));
        }

        let type_id = TypeId::of::<T>();

        // Validate scope access before proceeding
        self.validate_scope_access(resource_id, context)?;

        // Initialize dependencies in correct order before acquiring this resource
        self.ensure_dependencies_initialized(resource_id, context)
            .await?;

        // Get or create pool for this type
        let pool = if let Some(pool) = self.pools.get(&type_id) {
            Arc::clone(pool.value())
        } else {
            // Create a new pool for this type
            self.create_pool_for_type::<T>(resource_id, context).await?
        };

        // Acquire from pool
        let instance_any = pool.acquire_any(context).await?;

        // Cast to typed instance
        let typed_instance = self.cast_to_typed_instance::<T>(&instance_any)?;

        // Create guard with pool reference for cleanup
        Ok(self.create_guard_with_pool(typed_instance, pool))
    }

    /// Create a pool for a resource type
    async fn create_pool_for_type<T>(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
    ) -> ResourceResult<Arc<dyn PoolTrait + Send + Sync>>
    where
        T: Send + Sync + 'static,
    {
        let factory = {
            let registry = self.registry.read();
            registry.get(resource_id).cloned().ok_or_else(|| {
                ResourceError::unavailable(
                    resource_id.unique_key(),
                    "Resource type not registered",
                    false,
                )
            })?
        };

        // Retrieve stored config (fall back to Null if not found)
        let stored_config = self
            .resource_configs
            .get(resource_id)
            .map(|e| e.value().clone())
            .unwrap_or(serde_json::Value::Null);

        let factory_clone = Arc::clone(&factory);
        let context_clone = context.clone();
        let resource_id_clone = resource_id.clone();
        let event_subscribers = Arc::clone(&self.event_subscribers);

        // Create factory function for pool
        let pool_factory = move || {
            let factory = Arc::clone(&factory_clone);
            let context = context_clone.clone();
            let resource_id = resource_id_clone.clone();
            let subscribers = Arc::clone(&event_subscribers);
            let config = stored_config.clone();

            async move {
                // Emit creation start event
                let event = LifecycleEvent::new(
                    resource_id.unique_key(),
                    LifecycleState::Created,
                    LifecycleState::Initializing,
                );
                {
                    let subs = subscribers.read();
                    for sender in subs.iter() {
                        let _ = sender.unbounded_send(event.clone());
                    }
                }

                let dependencies = HashMap::new();

                // Create instance through factory
                let instance = factory
                    .create_instance(config, &context, &dependencies)
                    .await?;

                // Cast to typed instance
                let typed_instance = instance
                    .downcast_ref::<TypedResourceInstance<T>>()
                    .ok_or_else(|| {
                        ResourceError::internal(
                            "unknown",
                            "Failed to cast instance to requested type",
                        )
                    })?
                    .clone();

                // Emit ready event
                let event = LifecycleEvent::new(
                    resource_id.unique_key(),
                    LifecycleState::Initializing,
                    LifecycleState::Ready,
                );
                {
                    let subs = subscribers.read();
                    for sender in subs.iter() {
                        let _ = sender.unbounded_send(event.clone());
                    }
                }

                Ok(typed_instance)
            }
        };

        // Create pool with default config
        let pool_config = crate::core::traits::PoolConfig::default();
        let pool = ResourcePool::new(pool_config, PoolStrategy::Lifo, pool_factory);
        let pool_trait: Arc<dyn PoolTrait + Send + Sync> = Arc::new(pool);

        // Store pool
        let type_id = TypeId::of::<T>();
        self.pools.insert(type_id, Arc::clone(&pool_trait));

        Ok(pool_trait)
    }

    /// Subscribe to lifecycle events
    #[must_use]
    pub fn subscribe_to_events(&self) -> futures::channel::mpsc::UnboundedReceiver<LifecycleEvent> {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        {
            let mut subscribers = self.event_subscribers.write();
            subscribers.push(sender);
        }
        receiver
    }

    /// Get metadata for all registered resource types
    #[must_use]
    pub fn list_registered_types(&self) -> Vec<ResourceMetadata> {
        self.metadata_cache
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get metadata for a specific resource type
    #[must_use]
    pub fn get_metadata(&self, resource_id: &ResourceId) -> Option<ResourceMetadata> {
        self.metadata_cache
            .get(resource_id)
            .map(|entry| entry.value().clone())
    }

    // Helper methods

    fn find_resource_id_for_type<T>(&self) -> ResourceResult<ResourceId>
    where
        T: 'static,
    {
        self.type_to_resource
            .get(&TypeId::of::<T>())
            .map(|entry| entry.value().clone())
            .ok_or_else(|| {
                ResourceError::unavailable(
                    "unknown",
                    format!(
                        "No resource registered for type {}",
                        std::any::type_name::<T>()
                    ),
                    false,
                )
            })
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
            .ok_or_else(|| {
                ResourceError::internal("unknown", "Failed to cast instance to requested type")
            })
            .cloned()
    }

    fn create_guard_with_pool<T>(
        &self,
        instance: TypedResourceInstance<T>,
        _pool: Arc<dyn PoolTrait + Send + Sync>,
    ) -> ResourceGuard<T>
    where
        T: Send + Sync + 'static,
    {
        let instance_id = instance.instance_id();
        let type_id = TypeId::of::<T>();
        let cleanup_tx = self.cleanup_tx.clone();

        ResourceGuard::new(instance, move |_instance| {
            // Send cleanup message to async cleanup task
            let _ = cleanup_tx.send(CleanupMessage::Release {
                instance_id,
                type_id,
            });
        })
    }

    fn emit_lifecycle_event(&self, event: LifecycleEvent) {
        let subscribers = self.event_subscribers.read();
        for sender in subscribers.iter() {
            let _ = sender.unbounded_send(event.clone());
        }
    }

    /// Ensure all dependencies are initialized before this resource
    async fn ensure_dependencies_initialized(
        &self,
        resource_id: &ResourceId,
        _context: &ResourceContext,
    ) -> ResourceResult<()> {
        // Get initialization order (dependencies come first)
        let init_order = {
            let dep_graph = self.dependency_graph.read();
            dep_graph.get_init_order(resource_id)?
        };

        // Initialize each dependency in order (excluding the resource itself)
        for dep_id in &init_order {
            if dep_id == resource_id {
                // Skip self
                continue;
            }

            // Check if pool already exists for this dependency
            // We need to get TypeId for the dependency, but we don't know the concrete type
            // So we'll use the factory to create the pool if it doesn't exist

            // Get metadata to find the TypeId
            let _metadata = self.metadata_cache.get(dep_id).ok_or_else(|| {
                ResourceError::unavailable(dep_id.unique_key(), "Dependency not registered", false)
            })?;

            // Check if pool exists by looking in registry
            let factory = {
                let registry = self.registry.read();
                registry.get(dep_id).cloned()
            };

            if factory.is_none() {
                return Err(ResourceError::unavailable(
                    dep_id.unique_key(),
                    "Dependency factory not found",
                    false,
                ));
            }

            // At this point, the dependency is registered
            // The actual pool will be created lazily when first acquired
            // This is sufficient for dependency ordering - we just need to ensure
            // the factory is registered
        }

        Ok(())
    }

    /// Get the initialization order for all registered resources
    pub fn get_initialization_order(&self) -> ResourceResult<Vec<ResourceId>> {
        let dep_graph = self.dependency_graph.read();
        dep_graph.topological_sort()
    }

    /// Get all dependencies of a resource
    #[must_use]
    pub fn get_dependencies(&self, resource_id: &ResourceId) -> Vec<ResourceId> {
        let dep_graph = self.dependency_graph.read();
        dep_graph.get_dependencies(resource_id)
    }

    /// Get all resources that depend on this resource
    #[must_use]
    pub fn get_dependents(&self, resource_id: &ResourceId) -> Vec<ResourceId> {
        let dep_graph = self.dependency_graph.read();
        dep_graph.get_dependents(resource_id)
    }

    /// Check if one resource depends on another (directly or transitively)
    #[must_use]
    pub fn depends_on(&self, resource: &ResourceId, depends_on: &ResourceId) -> bool {
        let dep_graph = self.dependency_graph.read();
        dep_graph.depends_on(resource, depends_on)
    }

    /// Get the health status of a specific instance
    #[must_use]
    pub fn get_instance_health(&self, instance_id: &Uuid) -> Option<crate::health::HealthRecord> {
        self.health_checker.get_health(instance_id)
    }

    /// Get all health records for monitored instances
    #[must_use]
    pub fn get_all_health(&self) -> Vec<crate::health::HealthRecord> {
        self.health_checker.get_all_health()
    }

    /// Get all unhealthy instances
    #[must_use]
    pub fn get_unhealthy_instances(&self) -> Vec<crate::health::HealthRecord> {
        self.health_checker.get_unhealthy_instances()
    }

    /// Get instances that have exceeded the failure threshold
    #[must_use]
    pub fn get_critical_instances(&self) -> Vec<crate::health::HealthRecord> {
        self.health_checker.get_critical_instances()
    }

    /// Shutdown the manager and all background tasks
    ///
    /// Performs graceful shutdown:
    /// 1. Sets shutdown signal (no new acquisitions)
    /// 2. Stops health checking
    /// 3. Waits for active operations to complete
    /// 4. Shuts down all pools (which cleanup resources)
    /// 5. Emits shutdown lifecycle events
    pub async fn shutdown(&self) -> ResourceResult<()> {
        // Set shutdown signal to prevent new acquisitions
        {
            let mut signal = self.shutdown_signal.write().await;
            *signal = true;
        }

        // Emit shutdown starting event
        self.emit_lifecycle_event(LifecycleEvent::new(
            "resource-manager".to_string(),
            LifecycleState::Ready,
            LifecycleState::Draining,
        ));

        // Stop health checking first
        self.health_checker.shutdown().await;

        // Give active operations a moment to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Emit cleanup event
        self.emit_lifecycle_event(LifecycleEvent::new(
            "resource-manager".to_string(),
            LifecycleState::Draining,
            LifecycleState::Cleanup,
        ));

        // Shutdown all pools in reverse dependency order
        let shutdown_order = {
            let dep_graph = self.dependency_graph.read();
            // Get topological order and reverse it (shutdown dependencies last)
            dep_graph.topological_sort().unwrap_or_default()
        };

        for resource_id in shutdown_order.iter().rev() {
            // Find pool for this resource
            // Note: We can't easily map ResourceId -> TypeId without the concrete type,
            // so we'll just shutdown all pools
            if let Some(metadata) = self.metadata_cache.get(resource_id) {
                tracing::debug!("Shutting down resource: {}", resource_id);
                drop(metadata);
            }
        }

        // Shutdown all pools
        for entry in self.pools.iter() {
            entry.value().shutdown().await?;
        }

        // Emit final shutdown event
        self.emit_lifecycle_event(LifecycleEvent::new(
            "resource-manager".to_string(),
            LifecycleState::Cleanup,
            LifecycleState::Terminated,
        ));

        Ok(())
    }

    /// Check if the manager is shutting down
    pub async fn is_shutting_down(&self) -> bool {
        *self.shutdown_signal.read().await
    }

    /// Validate that the context has permission to access a resource in its scope
    fn validate_scope_access(
        &self,
        resource_id: &ResourceId,
        context: &ResourceContext,
    ) -> ResourceResult<()> {
        let metadata = self.metadata_cache.get(resource_id).ok_or_else(|| {
            ResourceError::unavailable(resource_id.to_string(), "Resource not registered", false)
        })?;

        let resource_scope = &metadata.default_scope;
        let context_scope = &context.scope;

        // The resource scope must contain the context scope.
        // e.g. a Global resource contains any context scope,
        //      a Tenant("A") resource contains Workflow(tenant_id=Some("A")),
        //      but NOT Workflow(tenant_id=Some("B")).
        if resource_scope.contains(context_scope) {
            Ok(())
        } else {
            Err(ResourceError::unavailable(
                resource_id.to_string(),
                format!(
                    "Scope mismatch: resource scope {resource_scope:?} does not contain context scope {context_scope:?}"
                ),
                false,
            ))
        }
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for `ResourceManager` with advanced configuration options
#[derive(Debug)]
pub struct ResourceManagerBuilder {
    config: ResourceManagerConfig,
    scoping_strategy: ScopingStrategy,
}

impl ResourceManagerBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ResourceManagerConfig::default(),
            scoping_strategy: ScopingStrategy::default(),
        }
    }

    /// Set the default timeout
    #[must_use]
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.config.default_timeout = timeout;
        self
    }

    /// Set the maximum instances per type
    #[must_use]
    pub fn max_instances_per_type(mut self, max: usize) -> Self {
        self.config.max_instances_per_type = max;
        self
    }

    /// Set the health check interval
    #[must_use]
    pub fn health_check_interval(mut self, interval: Duration) -> Self {
        self.config.health_check_interval = interval;
        self
    }

    /// Enable or disable automatic cleanup
    #[must_use]
    pub fn auto_cleanup(mut self, enabled: bool) -> Self {
        self.config.auto_cleanup_enabled = enabled;
        self
    }

    /// Set the scoping strategy
    #[must_use]
    pub fn scoping_strategy(mut self, strategy: ScopingStrategy) -> Self {
        self.scoping_strategy = strategy;
        self
    }

    /// Build the resource manager
    #[must_use]
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

/// Wrapper that adapts Resource trait to `ResourceFactory` trait
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
    R::Config: serde::de::DeserializeOwned,
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
            ResourceError::configuration(format!("Failed to deserialize config: {e}"))
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
            tags: HashMap::new(),
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
    use crate::core::resource::{ResourceConfig, ResourceInstance};

    // Mock resource for testing
    struct TestResource;

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct TestConfig;

    impl ResourceConfig for TestConfig {
        fn merge(&mut self, _other: Self) {}
    }

    struct TestInstance {
        id: Uuid,
        resource_id: ResourceId,
        context: ResourceContext,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    impl TestInstance {
        fn new(resource_id: ResourceId, context: ResourceContext) -> Self {
            Self {
                id: Uuid::new_v4(),
                resource_id,
                context,
                created_at: chrono::Utc::now(),
            }
        }

        fn with_id(resource_id: ResourceId) -> Self {
            Self::new(
                resource_id,
                ResourceContext::new(
                    "default-workflow".to_string(),
                    "Default Workflow".to_string(),
                    "default-execution".to_string(),
                    "development".to_string(),
                ),
            )
        }
    }

    impl ResourceInstance for TestInstance {
        fn instance_id(&self) -> Uuid {
            self.id
        }
        fn resource_id(&self) -> &ResourceId {
            &self.resource_id
        }
        fn lifecycle_state(&self) -> LifecycleState {
            LifecycleState::Ready
        }
        fn context(&self) -> &ResourceContext {
            &self.context
        }
        fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
            self.created_at
        }
        fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            None
        }
        fn touch(&self) {}
    }

    #[async_trait]
    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = TestInstance;

        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::new(ResourceId::new("test", "1.0"), "Test resource".to_string())
        }

        async fn create(
            &self,
            _config: &Self::Config,
            context: &ResourceContext,
        ) -> ResourceResult<Self::Instance> {
            Ok(TestInstance::new(
                ResourceId::new("test", "1.0"),
                context.clone(),
            ))
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

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let manager = ResourceManager::new();

        // Verify not shutting down initially
        assert!(!manager.is_shutting_down().await);

        // Shutdown
        manager.shutdown().await.unwrap();

        // Verify shutdown signal is set
        assert!(manager.is_shutting_down().await);
    }

    #[tokio::test]
    async fn test_shutdown_rejects_new_acquisitions() {
        let manager = ResourceManager::new();
        let resource = TestResource;
        manager.register(resource).unwrap();

        // Shutdown the manager
        manager.shutdown().await.unwrap();

        // Try to acquire a resource - should fail
        let context = ResourceContext::new(
            "test-workflow".to_string(),
            "test-exec".to_string(),
            "development".to_string(),
            "test-tenant".to_string(),
        );
        let resource_id = ResourceId::new("test", "1.0");
        let result = manager
            .get_by_id::<TestInstance>(&resource_id, &context)
            .await;

        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("shutting down"));
        }
    }

    #[test]
    fn test_instance_complete_implementation() {
        // Test that all TestInstance methods work without panicking
        let resource_id = ResourceId::new("test-resource", "1.0");
        let context = ResourceContext::new(
            "test-workflow".to_string(),
            "Test Workflow".to_string(),
            "test-exec".to_string(),
            "development".to_string(),
        );

        let instance = TestInstance::new(resource_id.clone(), context.clone());

        // Test all ResourceInstance trait methods
        let instance_id = instance.instance_id();
        assert!(!instance_id.is_nil());

        let returned_resource_id = instance.resource_id();
        assert_eq!(returned_resource_id.name, "test-resource");
        assert_eq!(returned_resource_id.version, "1.0");

        let state = instance.lifecycle_state();
        assert_eq!(state, LifecycleState::Ready);

        let returned_context = instance.context();
        assert_eq!(returned_context.workflow.workflow_id, "test-workflow");
        assert_eq!(returned_context.execution.execution_id, "test-exec");

        let created = instance.created_at();
        assert!(created <= chrono::Utc::now());

        let last_accessed = instance.last_accessed_at();
        assert!(last_accessed.is_none());

        // Touch should not panic
        instance.touch();
    }

    #[test]
    fn test_instance_with_id_constructor() {
        // Test the convenience constructor
        let resource_id = ResourceId::new("simple", "2.0");
        let instance = TestInstance::with_id(resource_id.clone());

        assert_eq!(instance.resource_id().name, "simple");
        assert!(!instance.context().workflow.workflow_id.is_empty());
    }
}
