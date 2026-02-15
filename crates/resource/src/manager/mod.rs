//! Resource manager — central registry and pool orchestration.

pub mod dependency;

use std::any::{Any, TypeId};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use crate::context::ResourceContext;
use crate::error::{ResourceError, ResourceResult};
use crate::pool::{Pool, PoolConfig};
use crate::resource::{Resource, ResourceGuard};

use dependency::DependencyGraph;

// ---------------------------------------------------------------------------
// Type-erased pool wrapper
// ---------------------------------------------------------------------------

/// Type-erased pool interface so the manager can store pools of different
/// resource types in a single map.
#[async_trait]
trait AnyPool: Send + Sync {
    /// Acquire a type-erased instance wrapped in `Arc<dyn Any>`.
    async fn acquire_any(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>>;

    /// Shut down the pool.
    async fn shutdown(&self) -> ResourceResult<()>;
}

/// Concrete adapter from `Pool<R>` to `AnyPool`.
struct TypedPool<R: Resource> {
    pool: Pool<R>,
}

#[async_trait]
impl<R: Resource> AnyPool for TypedPool<R> {
    async fn acquire_any(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>> {
        let guard = self.pool.acquire(ctx).await?;
        let instance: R::Instance = guard.into_inner();
        let arc_instance: Arc<dyn Any + Send + Sync> = Arc::new(instance);
        // No return-to-pool for the type-erased path — the instance is consumed.
        // For full pool recycling, callers should use Pool<R> directly.
        Ok(ResourceGuard::new(arc_instance, |_| {}))
    }

    async fn shutdown(&self) -> ResourceResult<()> {
        self.pool.shutdown().await
    }
}

// ---------------------------------------------------------------------------
// ResourceManager
// ---------------------------------------------------------------------------

/// Central manager for resource pools and dependency ordering.
pub struct ResourceManager {
    /// Pools indexed by TypeId of the Resource implementation.
    pools: DashMap<TypeId, Arc<dyn AnyPool>>,
    /// Dependency graph for initialization ordering.
    deps: parking_lot::RwLock<DependencyGraph>,
}

impl ResourceManager {
    /// Create a new empty resource manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pools: DashMap::new(),
            deps: parking_lot::RwLock::new(DependencyGraph::new()),
        }
    }

    /// Register a resource type with its config and pool settings.
    ///
    /// The pool is created immediately but no instances are pre-warmed.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
    ) -> ResourceResult<()> {
        let type_id = TypeId::of::<R>();
        let id = resource.id().to_string();

        // Register dependencies
        {
            let mut deps = self.deps.write();
            for dep in resource.dependencies() {
                deps.add_dependency(&id, dep)?;
            }
        }

        let pool = Pool::new(resource, config, pool_config);
        let any_pool: Arc<dyn AnyPool> = Arc::new(TypedPool { pool });
        self.pools.insert(type_id, any_pool);
        Ok(())
    }

    /// Acquire a resource instance by resource type.
    ///
    /// The caller specifies the Resource implementation type `R` and gets
    /// back a guard wrapping `R::Instance`.
    pub async fn acquire<R: Resource>(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>> {
        let type_id = TypeId::of::<R>();
        let pool = self.pools.get(&type_id).ok_or_else(|| {
            ResourceError::unavailable("unknown", "Resource type not registered", false)
        })?;

        pool.acquire_any(ctx).await
    }

    /// Get the initialization order based on dependency graph.
    pub fn initialization_order(&self) -> ResourceResult<Vec<String>> {
        self.deps.read().topological_sort()
    }

    /// Shut down all registered pools.
    pub async fn shutdown(&self) -> ResourceResult<()> {
        let pools: Vec<Arc<dyn AnyPool>> = self
            .pools
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();

        for pool in pools {
            pool.shutdown().await?;
        }

        self.pools.clear();
        Ok(())
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ResourceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceManager")
            .field("pool_count", &self.pools.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceConfig;
    use crate::scope::ResourceScope;

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig {
        value: String,
    }

    impl ResourceConfig for TestConfig {
        fn validate(&self) -> ResourceResult<()> {
            if self.value.is_empty() {
                return Err(ResourceError::configuration("value cannot be empty"));
            }
            Ok(())
        }
    }

    struct TestResource;

    #[async_trait]
    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            "test"
        }

        async fn create(
            &self,
            config: &Self::Config,
            _ctx: &ResourceContext,
        ) -> ResourceResult<Self::Instance> {
            Ok(format!("instance-{}", config.value))
        }
    }

    fn ctx() -> ResourceContext {
        ResourceContext::new(ResourceScope::Global, "wf", "ex")
    }

    #[tokio::test]
    async fn register_and_acquire() {
        let mgr = ResourceManager::new();
        let config = TestConfig {
            value: "hello".into(),
        };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();

        let guard = mgr.acquire::<TestResource>(&ctx()).await.unwrap();
        let instance = guard
            .downcast_ref::<String>()
            .expect("should downcast to String");
        assert_eq!(instance, "instance-hello");
    }

    #[tokio::test]
    async fn acquire_unregistered_fails() {
        let mgr = ResourceManager::new();
        let result = mgr.acquire::<TestResource>(&ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_clears_pools() {
        let mgr = ResourceManager::new();
        let config = TestConfig { value: "x".into() };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();
        mgr.shutdown().await.unwrap();
        assert!(mgr.pools.is_empty());
    }
}
