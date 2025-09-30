//! Resource pooling system

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::{ResourceId, TypedResourceInstance},
    traits::{HealthCheckable, HealthStatus, PoolConfig},
};

/// Strategy for selecting resources from the pool
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PoolStrategy {
    /// First In, First Out - maintains order, spreads load evenly
    Fifo,
    /// Last In, First Out - keeps recently used resources warm
    Lifo,
    /// Least Recently Used - evicts oldest used resources first
    Lru,
    /// Weighted Round Robin - considers resource health/performance
    WeightedRoundRobin,
    /// Adaptive - uses ML/heuristics to optimize selection
    Adaptive,
}

impl Default for PoolStrategy {
    fn default() -> Self {
        Self::Lifo
    }
}

/// Statistics about pool usage
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PoolStats {
    /// Total number of acquisitions
    pub total_acquisitions: u64,
    /// Total number of releases
    pub total_releases: u64,
    /// Current active (acquired) count
    pub active_count: usize,
    /// Current idle (available) count
    pub idle_count: usize,
    /// Peak active count
    pub peak_active_count: usize,
    /// Average acquisition time in milliseconds
    pub avg_acquisition_time_ms: f64,
    /// Number of failed acquisitions
    pub failed_acquisitions: u64,
    /// Number of resources created
    pub resources_created: u64,
    /// Number of resources destroyed
    pub resources_destroyed: u64,
    /// Last acquisition timestamp
    pub last_acquisition: Option<chrono::DateTime<chrono::Utc>>,
    /// Health check statistics
    pub health_checks: HealthCheckStats,
}

impl Default for PoolStats {
    fn default() -> Self {
        Self {
            total_acquisitions: 0,
            total_releases: 0,
            active_count: 0,
            idle_count: 0,
            peak_active_count: 0,
            avg_acquisition_time_ms: 0.0,
            failed_acquisitions: 0,
            resources_created: 0,
            resources_destroyed: 0,
            last_acquisition: None,
            health_checks: HealthCheckStats::default(),
        }
    }
}

/// Health check statistics for the pool
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HealthCheckStats {
    /// Total health checks performed
    pub total_checks: u64,
    /// Successful health checks
    pub successful_checks: u64,
    /// Failed health checks
    pub failed_checks: u64,
    /// Average health check duration in milliseconds
    pub avg_check_duration_ms: f64,
    /// Last health check timestamp
    pub last_check: Option<chrono::DateTime<chrono::Utc>>,
}

/// Entry in the resource pool
struct PoolEntry<T> {
    /// The resource instance
    instance: TypedResourceInstance<T>,
    /// When this entry was created
    created_at: Instant,
    /// When this entry was last accessed
    last_accessed: Instant,
    /// Number of times this entry has been acquired
    acquisition_count: u64,
    /// Last health check result
    last_health_check: Option<(Instant, HealthStatus)>,
}

impl<T> PoolEntry<T> {
    fn new(instance: TypedResourceInstance<T>) -> Self {
        let now = Instant::now();
        Self {
            instance,
            created_at: now,
            last_accessed: now,
            acquisition_count: 0,
            last_health_check: None,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.acquisition_count += 1;
    }

    fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    fn idle_time(&self) -> Duration {
        self.last_accessed.elapsed()
    }

    fn is_expired(&self, max_lifetime: Duration, idle_timeout: Duration) -> bool {
        self.age() > max_lifetime || self.idle_time() > idle_timeout
    }
}

/// Generic resource pool implementation
pub struct ResourcePool<T> {
    /// Pool configuration
    config: PoolConfig,
    /// Pool strategy
    strategy: PoolStrategy,
    /// Available resources
    available: Arc<Mutex<Vec<PoolEntry<T>>>>,
    /// Currently acquired resources
    acquired: Arc<Mutex<std::collections::HashMap<Uuid, PoolEntry<T>>>>,
    /// Pool statistics
    stats: Arc<RwLock<PoolStats>>,
    /// Factory function for creating new resources
    factory: Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ResourceResult<TypedResourceInstance<T>>> + Send>> + Send + Sync>,
    /// Resource health checker
    health_checker: Option<Arc<dyn HealthCheckable + Send + Sync>>,
}

impl<T> ResourcePool<T>
where
    T: Send + Sync + 'static,
{
    /// Create a new resource pool
    pub fn new<F, Fut>(config: PoolConfig, strategy: PoolStrategy, factory: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ResourceResult<TypedResourceInstance<T>>> + Send + 'static,
    {
        Self {
            config,
            strategy,
            available: Arc::new(Mutex::new(Vec::new())),
            acquired: Arc::new(Mutex::new(std::collections::HashMap::new())),
            stats: Arc::new(RwLock::new(PoolStats::default())),
            factory: Arc::new(move || Box::pin(factory())),
            health_checker: None,
        }
    }

    /// Set a health checker for the pool
    pub fn with_health_checker(mut self, checker: Arc<dyn HealthCheckable + Send + Sync>) -> Self {
        self.health_checker = Some(checker);
        self
    }

    /// Acquire a resource from the pool
    pub async fn acquire(&self) -> ResourceResult<PooledResource<T>> {
        let start_time = Instant::now();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_acquisitions += 1;
            stats.last_acquisition = Some(chrono::Utc::now());
        }

        // Try to get an existing resource
        if let Some(mut entry) = self.get_available_resource().await? {
            entry.touch();
            let instance_id = entry.instance.instance_id();

            // Move to acquired
            {
                let mut acquired = self.acquired.lock();
                acquired.insert(instance_id, entry);
            }

            // Update stats
            {
                let mut stats = self.stats.write();
                stats.active_count += 1;
                let duration = start_time.elapsed();
                stats.avg_acquisition_time_ms =
                    (stats.avg_acquisition_time_ms * (stats.total_acquisitions - 1) as f64 +
                     duration.as_millis() as f64) / stats.total_acquisitions as f64;
            }

            return Ok(PooledResource::new(instance_id, Arc::clone(&self.acquired), Arc::clone(&self.stats)));
        }

        // Create new resource if pool not at capacity
        {
            let stats = self.stats.read();
            if stats.active_count + stats.idle_count >= self.config.max_size {
                return Err(ResourceError::pool_exhausted(
                    "pool",
                    stats.active_count + stats.idle_count,
                    self.config.max_size,
                    0, // waiters - would need to implement waiting queue
                ));
            }
        }

        // Create new resource
        let instance = (self.factory)().await?;
        let mut entry = PoolEntry::new(instance);
        entry.touch();
        let instance_id = entry.instance.instance_id();

        // Add to acquired
        {
            let mut acquired = self.acquired.lock();
            acquired.insert(instance_id, entry);
        }

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.active_count += 1;
            stats.resources_created += 1;
            if stats.active_count > stats.peak_active_count {
                stats.peak_active_count = stats.active_count;
            }
            let duration = start_time.elapsed();
            stats.avg_acquisition_time_ms =
                (stats.avg_acquisition_time_ms * (stats.total_acquisitions - 1) as f64 +
                 duration.as_millis() as f64) / stats.total_acquisitions as f64;
        }

        Ok(PooledResource::new(instance_id, Arc::clone(&self.acquired), Arc::clone(&self.stats)))
    }

    /// Release a resource back to the pool
    pub async fn release(&self, instance_id: Uuid) -> ResourceResult<()> {
        let entry = {
            let mut acquired = self.acquired.lock();
            acquired.remove(&instance_id)
        };

        if let Some(entry) = entry {
            // Check if resource should be kept in pool
            if !entry.is_expired(self.config.max_lifetime, self.config.idle_timeout) {
                // Health check if enabled
                if let Some(health_checker) = &self.health_checker {
                    match health_checker.health_check().await {
                        Ok(health) if health.is_usable() => {
                            // Add to available pool
                            let mut available = self.available.lock();
                            available.push(entry);
                        }
                        _ => {
                            // Resource is unhealthy, destroy it
                            self.update_destroy_stats();
                        }
                    }
                } else {
                    // No health checker, add to pool
                    let mut available = self.available.lock();
                    available.push(entry);
                }
            } else {
                // Resource expired, destroy it
                self.update_destroy_stats();
            }

            // Update stats
            {
                let mut stats = self.stats.write();
                stats.total_releases += 1;
                stats.active_count = stats.active_count.saturating_sub(1);
                stats.idle_count = self.available.lock().len();
            }
        }

        Ok(())
    }

    /// Get current pool statistics
    pub fn stats(&self) -> PoolStats {
        self.stats.read().clone()
    }

    /// Perform maintenance on the pool (cleanup expired resources)
    pub async fn maintain(&self) -> ResourceResult<()> {
        let mut available = self.available.lock();
        let initial_count = available.len();

        // Remove expired resources
        available.retain(|entry| {
            !entry.is_expired(self.config.max_lifetime, self.config.idle_timeout)
        });

        let removed_count = initial_count - available.len();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.resources_destroyed += removed_count as u64;
            stats.idle_count = available.len();
        }

        // Ensure minimum pool size
        while available.len() < self.config.min_size {
            match (self.factory)().await {
                Ok(instance) => {
                    available.push(PoolEntry::new(instance));
                }
                Err(_) => break, // Can't create more resources
            }
        }

        Ok(())
    }

    /// Shutdown the pool and cleanup all resources
    pub async fn shutdown(&self) -> ResourceResult<()> {
        // Clear all resources
        {
            let mut available = self.available.lock();
            available.clear();
        }
        {
            let mut acquired = self.acquired.lock();
            acquired.clear();
        }

        Ok(())
    }

    // Helper methods

    async fn get_available_resource(&self) -> ResourceResult<Option<PoolEntry<T>>> {
        let mut available = self.available.lock();

        if available.is_empty() {
            return Ok(None);
        }

        let index = match self.strategy {
            PoolStrategy::Fifo => 0,
            PoolStrategy::Lifo => available.len() - 1,
            PoolStrategy::Lru => {
                // Find least recently used
                available
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.last_accessed)
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            PoolStrategy::WeightedRoundRobin => {
                // Simple implementation - would need more sophisticated logic
                0
            }
            PoolStrategy::Adaptive => {
                // Simple implementation - would need ML/heuristics
                0
            }
        };

        Ok(Some(available.remove(index)))
    }

    fn update_destroy_stats(&self) {
        let mut stats = self.stats.write();
        stats.resources_destroyed += 1;
    }
}

/// A resource that's been acquired from a pool
pub struct PooledResource<T> {
    /// Instance ID
    instance_id: Uuid,
    /// Reference to the acquired resources map
    acquired: Arc<Mutex<std::collections::HashMap<Uuid, PoolEntry<T>>>>,
    /// Reference to pool stats
    stats: Arc<RwLock<PoolStats>>,
}

impl<T> PooledResource<T> {
    fn new(
        instance_id: Uuid,
        acquired: Arc<Mutex<std::collections::HashMap<Uuid, PoolEntry<T>>>>,
        stats: Arc<RwLock<PoolStats>>,
    ) -> Self {
        Self {
            instance_id,
            acquired,
            stats,
        }
    }

    /// Get a reference to the underlying resource
    pub fn as_ref(&self) -> Option<&T> {
        let acquired = self.acquired.lock();
        acquired.get(&self.instance_id).map(|entry| entry.instance.as_ref())
    }

    /// Get the instance ID
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }
}

impl<T> Drop for PooledResource<T> {
    fn drop(&mut self) {
        // Note: In a real implementation, we'd need to handle the async release
        // This would typically involve sending the instance_id to a cleanup task
    }
}

/// Manager for multiple resource pools
pub struct PoolManager {
    /// Registered pools by resource ID
    pools: Arc<RwLock<std::collections::HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl PoolManager {
    /// Create a new pool manager
    pub fn new() -> Self {
        Self {
            pools: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Register a pool for a resource type
    pub fn register_pool<T>(&self, pool_id: String, pool: ResourcePool<T>)
    where
        T: Send + Sync + 'static,
    {
        let mut pools = self.pools.write();
        pools.insert(pool_id, Arc::new(pool));
    }

    /// Get a pool for a resource type
    pub fn get_pool<T>(&self, pool_id: &str) -> Option<Arc<ResourcePool<T>>>
    where
        T: Send + Sync + 'static,
    {
        let pools = self.pools.read();
        pools.get(pool_id)?.downcast_ref::<ResourcePool<T>>().map(|pool| {
            // This is a simplification - in reality we'd need better type handling
            unsafe { std::mem::transmute(pool) }
        })
    }

    /// Perform maintenance on all pools
    pub async fn maintain_all(&self) -> ResourceResult<()> {
        // TODO: Implement maintenance for all pools
        Ok(())
    }

    /// Shutdown all pools
    pub async fn shutdown_all(&self) -> ResourceResult<()> {
        // TODO: Implement shutdown for all pools
        Ok(())
    }
}

impl Default for PoolManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        context::ResourceContext,
        resource::{ResourceId, ResourceInstanceMetadata, TypedResourceInstance},
    };

    async fn create_test_instance() -> ResourceResult<TypedResourceInstance<String>> {
        let metadata = ResourceInstanceMetadata {
            instance_id: Uuid::new_v4(),
            resource_id: ResourceId::new("test", "1.0"),
            state: crate::core::lifecycle::LifecycleState::Ready,
            context: ResourceContext::new(
                "test".to_string(),
                "test".to_string(),
                "test".to_string(),
                "test".to_string(),
            ),
            created_at: chrono::Utc::now(),
            last_accessed_at: None,
            tags: std::collections::HashMap::new(),
        };

        Ok(TypedResourceInstance::new(
            Arc::new("test_resource".to_string()),
            metadata,
        ))
    }

    #[tokio::test]
    async fn test_pool_creation() {
        let config = PoolConfig::default();
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        let stats = pool.stats();
        assert_eq!(stats.total_acquisitions, 0);
        assert_eq!(stats.active_count, 0);
    }

    #[tokio::test]
    async fn test_pool_acquire_and_release() {
        let config = PoolConfig::default();
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Acquire a resource
        let resource = pool.acquire().await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.total_acquisitions, 1);
        assert_eq!(stats.active_count, 1);

        let instance_id = resource.instance_id();
        drop(resource);

        // Note: In the real implementation, dropping would trigger release
        // Here we manually release for testing
        pool.release(instance_id).await.unwrap();

        let stats = pool.stats();
        assert_eq!(stats.total_releases, 1);
        assert_eq!(stats.active_count, 0);
    }
}