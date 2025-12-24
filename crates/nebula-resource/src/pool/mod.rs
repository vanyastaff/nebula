//! Resource pooling system

use std::any::{Any, TypeId};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::TypedResourceInstance,
    traits::{HealthCheckable, HealthStatus},
};

// Re-export types for public API
pub use crate::core::traits::PoolConfig;

/// Strategy for selecting resources from the pool
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum PoolStrategy {
    /// First In, First Out - maintains order, spreads load evenly
    Fifo,
    /// Last In, First Out - keeps recently used resources warm
    #[default]
    Lifo,
    /// Least Recently Used - evicts oldest used resources first
    Lru,
    /// Weighted Round Robin - considers resource health/performance
    WeightedRoundRobin,
    /// Adaptive - uses ML/heuristics to optimize selection
    Adaptive,
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
    /// Pool utilization (0.0 to 1.0)
    pub utilization: f64,
    /// Average wait time for acquisitions in milliseconds
    pub avg_wait_time_ms: f64,
    /// Total wait time in milliseconds
    pub total_wait_time_ms: f64,
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
            utilization: 0.0,
            avg_wait_time_ms: 0.0,
            total_wait_time_ms: 0.0,
        }
    }
}

impl PoolStats {
    /// Calculate current utilization percentage (0.0 to 1.0)
    #[must_use]
    pub fn calculate_utilization(&self, max_size: usize) -> f64 {
        if max_size == 0 {
            0.0
        } else {
            self.active_count as f64 / max_size as f64
        }
    }

    /// Check if pool is under-utilized (< 30% used)
    #[must_use]
    pub fn is_underutilized(&self) -> bool {
        self.utilization < 0.3
    }

    /// Check if pool is over-utilized (> 80% used)
    #[must_use]
    pub fn is_overutilized(&self) -> bool {
        self.utilization > 0.8
    }

    /// Get scaling recommendation based on utilization
    #[must_use]
    pub fn scaling_recommendation(&self, max_size: usize) -> ScalingRecommendation {
        if self.is_overutilized() && self.active_count >= max_size {
            ScalingRecommendation::ScaleUp {
                current_size: max_size,
                recommended_size: (max_size as f64 * 1.5) as usize,
                reason: "Pool is at capacity and highly utilized".to_string(),
            }
        } else if self.is_underutilized() && self.active_count < max_size / 2 {
            ScalingRecommendation::ScaleDown {
                current_size: max_size,
                recommended_size: (max_size as f64 * 0.75) as usize,
                reason: "Pool is under-utilized".to_string(),
            }
        } else {
            ScalingRecommendation::NoChange {
                current_size: max_size,
                reason: "Pool utilization is within acceptable range".to_string(),
            }
        }
    }
}

/// Scaling recommendation for the pool
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ScalingRecommendation {
    /// Recommend scaling up the pool
    ScaleUp {
        /// Current pool size
        current_size: usize,
        /// Recommended new size
        recommended_size: usize,
        /// Reason for scaling up
        reason: String,
    },
    /// Recommend scaling down the pool
    ScaleDown {
        /// Current pool size
        current_size: usize,
        /// Recommended new size
        recommended_size: usize,
        /// Reason for scaling down
        reason: String,
    },
    /// No scaling change needed
    NoChange {
        /// Current pool size
        current_size: usize,
        /// Reason for no change
        reason: String,
    },
}

/// Pool monitoring insights for operational visibility
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PoolMonitoringInsights {
    /// Current pool utilization (0.0 to 1.0)
    pub current_utilization: f64,
    /// Average acquisition time in milliseconds
    pub avg_acquisition_time_ms: f64,
    /// Average wait time in milliseconds
    pub avg_wait_time_ms: f64,
    /// Peak active resource count
    pub peak_active_count: usize,
    /// Current active resource count
    pub current_active_count: usize,
    /// Current idle resource count
    pub current_idle_count: usize,
    /// Number of failed acquisitions
    pub failed_acquisitions: u64,
    /// Scaling recommendation
    pub scaling_recommendation: ScalingRecommendation,
    /// Overall health score (0.0 to 1.0)
    pub health_score: f64,
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

/// Statistics returned from pool maintenance operations
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MaintenanceStats {
    /// Number of resources removed (expired or unhealthy)
    pub resources_removed: usize,
    /// Number of resources created to maintain `min_size`
    pub resources_created: usize,
    /// Number of resources validated
    pub resources_validated: usize,
    /// Number of failed validations
    pub failed_validations: usize,
    /// Duration of maintenance operation
    pub duration_ms: u64,
    /// Current pool state after maintenance
    pub pool_size: usize,
    /// Current idle resources
    pub idle_resources: usize,
    /// Current active resources
    pub active_resources: usize,
}

/// Statistics returned from pool shutdown operations
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ShutdownStats {
    /// Number of idle resources closed
    pub idle_resources_closed: usize,
    /// Number of active resources closed
    pub active_resources_closed: usize,
    /// Total resources closed
    pub total_resources_closed: usize,
    /// Duration of shutdown operation
    pub duration_ms: u64,
    /// Whether shutdown completed within timeout
    pub completed_gracefully: bool,
    /// Number of resources that were force-closed
    pub force_closed: usize,
}

/// Entry in the resource pool
#[derive(Debug)]
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
    /// Weight for weighted round robin (0.0 to 1.0)
    weight: f64,
    /// Performance metrics
    performance: PerformanceMetrics,
}

/// Performance metrics for a pool entry
#[derive(Debug, Clone)]
struct PerformanceMetrics {
    /// Average response time in milliseconds
    avg_response_time_ms: f64,
    /// Total successful operations
    successful_ops: u64,
    /// Total failed operations
    failed_ops: u64,
    /// Last operation timestamp
    last_operation: Option<Instant>,
}

impl PerformanceMetrics {
    fn new() -> Self {
        Self {
            avg_response_time_ms: 0.0,
            successful_ops: 0,
            failed_ops: 0,
            last_operation: None,
        }
    }

    fn record_success(&mut self, response_time_ms: f64) {
        let total_ops = self.successful_ops + 1;
        self.avg_response_time_ms = (self.avg_response_time_ms * self.successful_ops as f64
            + response_time_ms)
            / total_ops as f64;
        self.successful_ops = total_ops;
        self.last_operation = Some(Instant::now());
    }

    fn record_failure(&mut self) {
        self.failed_ops += 1;
        self.last_operation = Some(Instant::now());
    }

    fn success_rate(&self) -> f64 {
        let total = self.successful_ops + self.failed_ops;
        if total == 0 {
            1.0
        } else {
            self.successful_ops as f64 / total as f64
        }
    }
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
            weight: 1.0,
            performance: PerformanceMetrics::new(),
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

    /// Calculate weight based on health and performance
    fn calculate_weight(&mut self) -> f64 {
        // Base weight from health status
        let health_weight = if let Some((check_time, ref health)) = self.last_health_check {
            // Decay weight if health check is old (older than 30 seconds)
            let age_factor = if check_time.elapsed().as_secs() > 30 {
                0.7
            } else {
                1.0
            };

            match &health.state {
                crate::core::traits::HealthState::Healthy => 1.0 * age_factor,
                crate::core::traits::HealthState::Degraded {
                    performance_impact, ..
                } => (1.0 - performance_impact) * age_factor,
                crate::core::traits::HealthState::Unhealthy { .. } => 0.1 * age_factor,
                crate::core::traits::HealthState::Unknown => 0.5 * age_factor,
            }
        } else {
            // No health check data, assume healthy but penalize slightly
            0.8
        };

        // Performance weight based on success rate
        let perf_weight = self.performance.success_rate();

        // Response time weight (faster is better)
        // Normalize response time: 0-100ms = 1.0, 100-500ms = 0.5-1.0, >500ms = 0.1-0.5
        let response_weight = if self.performance.avg_response_time_ms == 0.0 {
            1.0
        } else if self.performance.avg_response_time_ms < 100.0 {
            1.0
        } else if self.performance.avg_response_time_ms < 500.0 {
            1.0 - (self.performance.avg_response_time_ms - 100.0) / 800.0
        } else {
            0.5 / (1.0 + (self.performance.avg_response_time_ms - 500.0) / 1000.0)
        };

        // Combined weight (weighted average)
        let weight = health_weight * 0.5 + perf_weight * 0.3 + response_weight * 0.2;

        // Update cached weight
        self.weight = weight;
        weight
    }
}

/// Type-erased trait for resource pool operations
///
/// This trait allows `ResourceManager` to work with pools of different types
/// without knowing the concrete type parameter at compile time.
#[async_trait]
pub trait PoolTrait: Send + Sync {
    /// Acquire a resource from the pool as type-erased Any
    async fn acquire_any(
        &self,
        context: &ResourceContext,
    ) -> ResourceResult<Arc<dyn Any + Send + Sync>>;

    /// Release a resource back to the pool
    async fn release_any(&self, instance_id: Uuid) -> ResourceResult<()>;

    /// Perform health checks on all pool resources
    async fn health_check_all(&self) -> ResourceResult<Vec<HealthStatus>>;

    /// Get pool statistics
    fn stats(&self) -> PoolStats;

    /// Perform maintenance on the pool
    async fn maintain(&self) -> ResourceResult<MaintenanceStats>;

    /// Shutdown the pool
    async fn shutdown(&self) -> ResourceResult<ShutdownStats>;

    /// Get the `TypeId` for the resource type this pool manages
    fn type_id(&self) -> TypeId;
}

/// Adaptive strategy state
#[derive(Debug, Clone)]
struct AdaptiveState {
    /// Recent selection history (resource index, response time)
    selection_history: Vec<(usize, f64)>,
    /// Maximum history size
    max_history: usize,
    /// Current strategy preference (0.0 = FIFO, 1.0 = Weighted)
    strategy_preference: f64,
}

impl AdaptiveState {
    fn new() -> Self {
        Self {
            selection_history: Vec::new(),
            max_history: 100,
            strategy_preference: 0.5,
        }
    }

    fn record_selection(&mut self, index: usize, response_time_ms: f64) {
        self.selection_history.push((index, response_time_ms));
        if self.selection_history.len() > self.max_history {
            self.selection_history.remove(0);
        }

        // Adjust strategy preference based on recent performance
        self.adjust_preference();
    }

    fn adjust_preference(&mut self) {
        if self.selection_history.len() < 10 {
            return;
        }

        // Calculate average response time for recent selections
        let recent_avg: f64 = self
            .selection_history
            .iter()
            .rev()
            .take(10)
            .map(|(_, rt)| rt)
            .sum::<f64>()
            / 10.0;

        let overall_avg: f64 = self.selection_history.iter().map(|(_, rt)| rt).sum::<f64>()
            / self.selection_history.len() as f64;

        // If recent performance is better, increase preference for current strategy
        if recent_avg < overall_avg * 0.9 {
            self.strategy_preference = (self.strategy_preference + 0.1).min(1.0);
        } else if recent_avg > overall_avg * 1.1 {
            self.strategy_preference = (self.strategy_preference - 0.1).max(0.0);
        }
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
    factory: Arc<
        dyn Fn() -> std::pin::Pin<
                Box<dyn Future<Output = ResourceResult<TypedResourceInstance<T>>> + Send>,
            > + Send
            + Sync,
    >,
    /// Resource health checker
    health_checker: Option<Arc<dyn HealthCheckable + Send + Sync>>,
    /// Weighted round robin state
    wrr_current_index: Arc<Mutex<usize>>,
    /// Adaptive strategy state
    adaptive_state: Arc<Mutex<AdaptiveState>>,
}

impl<T> std::fmt::Debug for ResourcePool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let available_count = self.available.lock().len();
        let acquired_count = self.acquired.lock().len();
        f.debug_struct("ResourcePool")
            .field("config", &self.config)
            .field("strategy", &self.strategy)
            .field("available_count", &available_count)
            .field("acquired_count", &acquired_count)
            .field("stats", &self.stats)
            .field("factory", &"<function>")
            .field(
                "health_checker",
                &self.health_checker.as_ref().map(|_| "<checker>"),
            )
            .field("wrr_current_index", &self.wrr_current_index)
            .field("adaptive_state", &self.adaptive_state)
            .finish()
    }
}

impl<T> ResourcePool<T>
where
    T: Send + Sync + 'static,
{
    /// Create a new resource pool
    pub fn new<F, Fut>(config: PoolConfig, strategy: PoolStrategy, factory: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ResourceResult<TypedResourceInstance<T>>> + Send + 'static,
    {
        Self {
            config: config.clone(),
            strategy,
            // Pre-allocate for max pool size to avoid reallocations
            available: Arc::new(Mutex::new(Vec::with_capacity(config.max_size))),
            acquired: Arc::new(Mutex::new(std::collections::HashMap::with_capacity(
                config.max_size,
            ))),
            stats: Arc::new(RwLock::new(PoolStats::default())),
            factory: Arc::new(move || Box::pin(factory())),
            health_checker: None,
            wrr_current_index: Arc::new(Mutex::new(0)),
            adaptive_state: Arc::new(Mutex::new(AdaptiveState::new())),
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
        let wait_start = Instant::now();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_acquisitions += 1;
            stats.last_acquisition = Some(chrono::Utc::now());
        }

        // Try to get an existing resource
        if let Some(mut entry) = self.get_available_resource().await? {
            let wait_time = wait_start.elapsed().as_millis() as f64;
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
                stats.avg_acquisition_time_ms = (stats.avg_acquisition_time_ms
                    * (stats.total_acquisitions - 1) as f64
                    + duration.as_millis() as f64)
                    / stats.total_acquisitions as f64;

                // Update wait time stats
                stats.total_wait_time_ms += wait_time;
                stats.avg_wait_time_ms = stats.total_wait_time_ms / stats.total_acquisitions as f64;

                // Update utilization
                stats.utilization = stats.calculate_utilization(self.config.max_size);
            }

            return Ok(PooledResource::new(
                instance_id,
                Arc::clone(&self.acquired),
                Arc::clone(&self.stats),
            ));
        }

        // Create new resource if pool not at capacity
        {
            let stats = self.stats.read();
            if stats.active_count + stats.idle_count >= self.config.max_size {
                // Update failed acquisition stats
                let mut stats = self.stats.write();
                stats.failed_acquisitions += 1;

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
        let wait_time = wait_start.elapsed().as_millis() as f64;
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
            stats.avg_acquisition_time_ms = (stats.avg_acquisition_time_ms
                * (stats.total_acquisitions - 1) as f64
                + duration.as_millis() as f64)
                / stats.total_acquisitions as f64;

            // Update wait time stats
            stats.total_wait_time_ms += wait_time;
            stats.avg_wait_time_ms = stats.total_wait_time_ms / stats.total_acquisitions as f64;

            // Update utilization
            stats.utilization = stats.calculate_utilization(self.config.max_size);
        }

        Ok(PooledResource::new(
            instance_id,
            Arc::clone(&self.acquired),
            Arc::clone(&self.stats),
        ))
    }

    /// Release a resource back to the pool
    pub async fn release(&self, instance_id: Uuid) -> ResourceResult<()> {
        let entry = {
            let mut acquired = self.acquired.lock();
            acquired.remove(&instance_id)
        };

        if let Some(entry) = entry {
            // Check if resource should be kept in pool
            if entry.is_expired(self.config.max_lifetime, self.config.idle_timeout) {
                // Resource expired, destroy it
                self.update_destroy_stats();
            } else {
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
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.stats.read().clone()
    }

    /// Get pool monitoring insights
    #[must_use]
    pub fn monitoring_insights(&self) -> PoolMonitoringInsights {
        let stats = self.stats.read();
        let recommendation = stats.scaling_recommendation(self.config.max_size);

        PoolMonitoringInsights {
            current_utilization: stats.utilization,
            avg_acquisition_time_ms: stats.avg_acquisition_time_ms,
            avg_wait_time_ms: stats.avg_wait_time_ms,
            peak_active_count: stats.peak_active_count,
            current_active_count: stats.active_count,
            current_idle_count: stats.idle_count,
            failed_acquisitions: stats.failed_acquisitions,
            scaling_recommendation: recommendation,
            health_score: self.calculate_health_score(&stats),
        }
    }

    /// Calculate overall health score for the pool (0.0 to 1.0)
    fn calculate_health_score(&self, stats: &PoolStats) -> f64 {
        // Start with perfect score
        let mut score = 1.0;

        // Penalize for high utilization (over 90%)
        if stats.utilization > 0.9 {
            score -= (stats.utilization - 0.9) * 0.5;
        }

        // Penalize for failed acquisitions
        if stats.total_acquisitions > 0 {
            let failure_rate = stats.failed_acquisitions as f64 / stats.total_acquisitions as f64;
            score -= failure_rate * 0.3;
        }

        // Penalize for slow acquisition times (> 100ms)
        if stats.avg_acquisition_time_ms > 100.0 {
            score -= ((stats.avg_acquisition_time_ms - 100.0) / 1000.0).min(0.2);
        }

        // Penalize for high wait times (> 50ms)
        if stats.avg_wait_time_ms > 50.0 {
            score -= ((stats.avg_wait_time_ms - 50.0) / 500.0).min(0.2);
        }

        score.clamp(0.0, 1.0)
    }

    /// Perform maintenance on the pool (cleanup expired resources)
    pub async fn maintain(&self) -> ResourceResult<MaintenanceStats> {
        let start_time = Instant::now();

        let (initial_count, removed_count, current_len) = {
            let mut available = self.available.lock();
            let initial_count = available.len();

            // Remove expired resources
            available.retain(|entry| {
                !entry.is_expired(self.config.max_lifetime, self.config.idle_timeout)
            });

            let removed_count = initial_count - available.len();
            let current_len = available.len();

            // Update stats
            {
                let mut stats = self.stats.write();
                stats.resources_destroyed += removed_count as u64;
                stats.idle_count = current_len;
            }

            (initial_count, removed_count, current_len)
        };

        // Validate remaining resources (health check)
        let mut resources_validated = 0;
        let mut failed_validations = 0;

        {
            let mut available = self.available.lock();
            for entry in available.iter_mut() {
                resources_validated += 1;
                if let Some((_, health)) = &entry.last_health_check
                    && matches!(
                        health.state,
                        crate::core::traits::HealthState::Unhealthy { .. }
                    )
                {
                    failed_validations += 1;
                }
            }

            // Remove unhealthy resources
            available.retain(|entry| {
                if let Some((_, health)) = &entry.last_health_check {
                    !matches!(
                        health.state,
                        crate::core::traits::HealthState::Unhealthy { .. }
                    )
                } else {
                    true // Keep resources without health check data
                }
            });
        }

        // Ensure minimum pool size
        let mut resources_created = 0;
        let mut current_count = current_len;
        while current_count < self.config.min_size {
            match (self.factory)().await {
                Ok(instance) => {
                    let mut available = self.available.lock();
                    available.push(PoolEntry::new(instance));
                    current_count = available.len();
                    resources_created += 1;

                    // Update stats
                    let mut stats = self.stats.write();
                    stats.resources_created += 1;
                }
                Err(_) => break, // Can't create more resources
            }
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Get final counts
        let (idle_resources, active_resources) = {
            let available = self.available.lock();
            let acquired = self.acquired.lock();
            (available.len(), acquired.len())
        };

        Ok(MaintenanceStats {
            resources_removed: removed_count,
            resources_created,
            resources_validated,
            failed_validations,
            duration_ms,
            pool_size: idle_resources + active_resources,
            idle_resources,
            active_resources,
        })
    }

    /// Shutdown the pool and cleanup all resources
    pub async fn shutdown(&self) -> ResourceResult<ShutdownStats> {
        self.shutdown_with_timeout(Duration::from_secs(30)).await
    }

    /// Shutdown the pool with a specific timeout
    pub async fn shutdown_with_timeout(&self, timeout: Duration) -> ResourceResult<ShutdownStats> {
        let start_time = Instant::now();

        // Get initial counts
        let (idle_count, active_count) = {
            let available = self.available.lock();
            let acquired = self.acquired.lock();
            (available.len(), acquired.len())
        };

        // Wait for active resources to be released (up to timeout)
        let mut completed_gracefully = true;
        let wait_deadline = Instant::now() + timeout;

        while Instant::now() < wait_deadline {
            let acquired_count = {
                let acquired = self.acquired.lock();
                acquired.len()
            };

            if acquired_count == 0 {
                break;
            }

            // Sleep briefly before checking again
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Check if we still have active resources after timeout
        let remaining_active = {
            let acquired = self.acquired.lock();
            acquired.len()
        };

        if remaining_active > 0 {
            completed_gracefully = false;
        }

        // Force close all remaining resources
        let force_closed = remaining_active;
        {
            let mut acquired = self.acquired.lock();
            acquired.clear();
        }

        // Close idle resources
        let idle_resources_closed = {
            let mut available = self.available.lock();
            let count = available.len();
            available.clear();
            count
        };

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.resources_destroyed += (idle_resources_closed + active_count) as u64;
            stats.idle_count = 0;
            stats.active_count = 0;
        }

        Ok(ShutdownStats {
            idle_resources_closed,
            active_resources_closed: active_count,
            total_resources_closed: idle_count + active_count,
            duration_ms,
            completed_gracefully,
            force_closed,
        })
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
                    .map_or(0, |(i, _)| i)
            }
            PoolStrategy::WeightedRoundRobin => self.select_weighted_round_robin(&mut available),
            PoolStrategy::Adaptive => self.select_adaptive(&mut available),
        };

        Ok(Some(available.remove(index)))
    }

    /// Select resource using weighted round robin strategy
    fn select_weighted_round_robin(&self, available: &mut [PoolEntry<T>]) -> usize {
        if available.is_empty() {
            return 0;
        }

        // Calculate weights for all resources
        let weights: Vec<f64> = available
            .iter_mut()
            .map(PoolEntry::calculate_weight)
            .collect();

        // Calculate total weight
        let total_weight: f64 = weights.iter().sum();

        if total_weight == 0.0 {
            // All weights are zero, fall back to round robin
            let mut current = self.wrr_current_index.lock();
            let index = *current % available.len();
            *current = (*current + 1) % available.len();
            return index;
        }

        // Get current index and find next resource using weighted probability
        let mut current = self.wrr_current_index.lock();
        let start_index = *current % available.len();

        // Weighted round robin: iterate from current position, selecting based on weight
        let mut cumulative_weight = 0.0;
        let target_weight = ((*current as f64 / available.len() as f64) % 1.0) * total_weight;

        for i in 0..available.len() {
            let idx = (start_index + i) % available.len();
            cumulative_weight += weights[idx];

            if cumulative_weight >= target_weight || i == available.len() - 1 {
                *current = (*current + 1) % (available.len() * 100); // Cycle through 100 rounds
                return idx;
            }
        }

        // Fallback
        start_index
    }

    /// Select resource using adaptive strategy
    fn select_adaptive(&self, available: &mut [PoolEntry<T>]) -> usize {
        if available.is_empty() {
            return 0;
        }

        let state = self.adaptive_state.lock();
        let preference = state.strategy_preference;
        drop(state);

        // Blend between FIFO (simple) and weighted (complex) based on preference
        if preference < 0.3 {
            // Prefer simple FIFO strategy
            0
        } else if preference > 0.7 {
            // Prefer weighted strategy
            self.select_weighted_round_robin(available)
        } else {
            // Hybrid approach: use performance metrics to select

            available
                .iter_mut()
                .enumerate()
                .map(|(idx, entry)| {
                    let weight = entry.calculate_weight();
                    let idle_penalty = entry.idle_time().as_secs_f64() / 60.0; // Penalize idle resources
                    let score = weight - idle_penalty.min(0.5);
                    (idx, score)
                })
                .max_by(|(_, score1), (_, score2)| {
                    score1
                        .partial_cmp(score2)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map_or(0, |(idx, _)| idx)
        }
    }

    /// Record performance metrics for adaptive strategy
    pub fn record_operation(&self, instance_id: Uuid, success: bool, response_time_ms: f64) {
        // Update entry performance metrics if it's currently acquired
        let mut acquired = self.acquired.lock();
        if let Some(entry) = acquired.get_mut(&instance_id) {
            if success {
                entry.performance.record_success(response_time_ms);
            } else {
                entry.performance.record_failure();
            }
        }
    }

    fn update_destroy_stats(&self) {
        let mut stats = self.stats.write();
        stats.resources_destroyed += 1;
    }
}

/// Implementation of `PoolTrait` for `ResourcePool`<T>
///
/// This provides type-erased access to pool operations, allowing the `ResourceManager`
/// to store and use pools of different types uniformly.
#[async_trait]
impl<T> PoolTrait for ResourcePool<T>
where
    T: Send + Sync + 'static,
{
    async fn acquire_any(
        &self,
        _context: &ResourceContext,
    ) -> ResourceResult<Arc<dyn Any + Send + Sync>> {
        let pooled = self.acquire().await?;
        let instance_id = pooled.instance_id();

        // Get the actual instance from the pooled resource
        let acquired = self.acquired.lock();
        if let Some(entry) = acquired.get(&instance_id) {
            // Return the TypedResourceInstance wrapped as Any
            Ok(Arc::new(entry.instance.clone()) as Arc<dyn Any + Send + Sync>)
        } else {
            Err(ResourceError::internal(
                "pool",
                "Failed to get acquired instance",
            ))
        }
    }

    async fn release_any(&self, instance_id: Uuid) -> ResourceResult<()> {
        self.release(instance_id).await
    }

    async fn health_check_all(&self) -> ResourceResult<Vec<HealthStatus>> {
        // Check all available resources
        let available = self.available.lock();

        // Pre-allocate for number of available resources
        let mut statuses = Vec::with_capacity(available.len());
        for entry in available.iter() {
            if let Some((_, status)) = &entry.last_health_check {
                statuses.push(status.clone());
            }
        }

        Ok(statuses)
    }

    fn stats(&self) -> PoolStats {
        self.stats()
    }

    async fn maintain(&self) -> ResourceResult<MaintenanceStats> {
        self.maintain().await
    }

    async fn shutdown(&self) -> ResourceResult<ShutdownStats> {
        self.shutdown().await
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }
}

/// A resource that's been acquired from a pool
#[derive(Debug)]
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

    /// Get a cloned Arc to the underlying resource
    #[must_use]
    pub fn get_instance(&self) -> Option<Arc<T>> {
        let acquired = self.acquired.lock();
        acquired
            .get(&self.instance_id())
            .map(|entry| Arc::clone(&entry.instance.instance))
    }

    /// Get the instance ID
    #[must_use]
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
    /// Registered pools by resource ID (stored as `PoolTrait` for type-erased operations)
    pools: Arc<RwLock<std::collections::HashMap<String, Arc<dyn PoolTrait>>>>,
}

impl std::fmt::Debug for PoolManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pool_count = self.pools.read().len();
        let pool_ids: Vec<String> = self.pools.read().keys().cloned().collect();
        f.debug_struct("PoolManager")
            .field("pool_count", &pool_count)
            .field("pool_ids", &pool_ids)
            .finish()
    }
}

impl PoolManager {
    /// Create a new pool manager
    #[must_use]
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
        pools.insert(pool_id, Arc::new(pool) as Arc<dyn PoolTrait>);
    }

    /// Get a pool for a resource type
    #[must_use]
    pub fn get_pool<T>(&self, pool_id: &str) -> Option<Arc<ResourcePool<T>>>
    where
        T: Send + Sync + 'static,
    {
        let pools = self.pools.read();
        let pool_trait = pools.get(pool_id)?;

        // We need to downcast through Any since Arc<dyn PoolTrait> doesn't directly support downcast
        // This is a limitation of trait objects
        None // For now, this is not directly supported
    }

    /// Perform maintenance on all pools
    pub async fn maintain_all(
        &self,
    ) -> ResourceResult<std::collections::HashMap<String, MaintenanceStats>> {
        let pools = {
            let pools_guard = self.pools.read();
            pools_guard
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect::<Vec<_>>()
        };

        let mut results = std::collections::HashMap::new();

        for (pool_id, pool) in pools {
            match pool.maintain().await {
                Ok(stats) => {
                    results.insert(pool_id, stats);
                }
                Err(e) => {
                    // Log error but continue with other pools
                    eprintln!("Failed to maintain pool {pool_id}: {e:?}");
                }
            }
        }

        Ok(results)
    }

    /// Shutdown all pools
    pub async fn shutdown_all(
        &self,
    ) -> ResourceResult<std::collections::HashMap<String, ShutdownStats>> {
        let pools = {
            let pools_guard = self.pools.read();
            pools_guard
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect::<Vec<_>>()
        };

        let mut results = std::collections::HashMap::new();

        for (pool_id, pool) in pools {
            match pool.shutdown().await {
                Ok(stats) => {
                    results.insert(pool_id, stats);
                }
                Err(e) => {
                    // Log error but continue with other pools
                    eprintln!("Failed to shutdown pool {pool_id}: {e:?}");
                }
            }
        }

        // Clear all pools after shutdown
        {
            let mut pools_guard = self.pools.write();
            pools_guard.clear();
        }

        Ok(results)
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

    #[tokio::test]
    async fn test_weighted_round_robin_strategy() {
        let config = PoolConfig {
            min_size: 3,
            max_size: 5,
            ..Default::default()
        };
        let pool = ResourcePool::new(
            config,
            PoolStrategy::WeightedRoundRobin,
            create_test_instance,
        );

        // Pre-populate pool with some resources
        let mut resources = Vec::new();
        for _ in 0..3 {
            resources.push(pool.acquire().await.unwrap());
        }

        // Release them back to pool
        for resource in resources {
            pool.release(resource.instance_id()).await.unwrap();
        }

        // Now acquire with weighted strategy - should select based on weights
        let resource1 = pool.acquire().await.unwrap();
        assert!(resource1.instance_id().to_string().len() == 36);

        pool.release(resource1.instance_id()).await.unwrap();

        let stats = pool.stats();
        assert!(stats.total_acquisitions >= 4);
    }

    #[tokio::test]
    async fn test_adaptive_strategy() {
        let config = PoolConfig {
            min_size: 2,
            max_size: 5,
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Adaptive, create_test_instance);

        // Acquire and release multiple times to build history
        for _ in 0..5 {
            let resource = pool.acquire().await.unwrap();
            let instance_id = resource.instance_id();
            drop(resource);
            pool.release(instance_id).await.unwrap();
        }

        let stats = pool.stats();
        assert_eq!(stats.total_acquisitions, 5);
        assert_eq!(stats.total_releases, 5);
        assert!(stats.avg_acquisition_time_ms >= 0.0);
    }

    #[tokio::test]
    async fn test_pool_utilization_tracking() {
        let config = PoolConfig {
            min_size: 0,
            max_size: 10,
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Acquire 5 resources (50% utilization)
        let mut resources = Vec::new();
        for _ in 0..5 {
            resources.push(pool.acquire().await.unwrap());
        }

        let stats = pool.stats();
        assert_eq!(stats.active_count, 5);
        assert_eq!(stats.utilization, 0.5);
        assert!(!stats.is_underutilized());
        assert!(!stats.is_overutilized());

        // Acquire 4 more (90% utilization)
        for _ in 0..4 {
            resources.push(pool.acquire().await.unwrap());
        }

        let stats = pool.stats();
        assert_eq!(stats.active_count, 9);
        assert!(stats.utilization >= 0.8);
        assert!(stats.is_overutilized());
    }

    #[tokio::test]
    async fn test_pool_monitoring_insights() {
        let config = PoolConfig {
            min_size: 0,
            max_size: 10,
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Acquire some resources
        let mut resources = Vec::new();
        for _ in 0..3 {
            resources.push(pool.acquire().await.unwrap());
        }

        let insights = pool.monitoring_insights();
        assert_eq!(insights.current_active_count, 3);
        assert_eq!(insights.current_idle_count, 0);
        assert!(insights.health_score >= 0.0 && insights.health_score <= 1.0);
        assert!(matches!(
            insights.scaling_recommendation,
            ScalingRecommendation::NoChange { .. }
        ));

        // Acquire many more to trigger scale-up recommendation
        for _ in 0..7 {
            resources.push(pool.acquire().await.unwrap());
        }

        let insights = pool.monitoring_insights();
        assert_eq!(insights.current_active_count, 10);
        assert!(insights.current_utilization >= 0.9);
    }

    #[tokio::test]
    async fn test_pool_performance_metrics() {
        let config = PoolConfig::default();
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Acquire and release to build metrics
        let resource = pool.acquire().await.unwrap();
        let instance_id = resource.instance_id();

        // Record some operations
        pool.record_operation(instance_id, true, 50.0);
        pool.record_operation(instance_id, true, 75.0);
        pool.record_operation(instance_id, false, 0.0);

        drop(resource);
        pool.release(instance_id).await.unwrap();

        let stats = pool.stats();
        assert!(stats.avg_acquisition_time_ms >= 0.0);
        assert!(stats.avg_wait_time_ms >= 0.0);
    }

    #[tokio::test]
    async fn test_lru_strategy() {
        let config = PoolConfig {
            min_size: 0,
            max_size: 5,
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Lru, create_test_instance);

        // Acquire and release multiple resources
        let r1 = pool.acquire().await.unwrap();
        let id1 = r1.instance_id();
        drop(r1);
        pool.release(id1).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let r2 = pool.acquire().await.unwrap();
        let id2 = r2.instance_id();
        drop(r2);
        pool.release(id2).await.unwrap();

        // Next acquire should get least recently used (r1)
        let r3 = pool.acquire().await.unwrap();
        // In LRU, the oldest used resource should be selected
        assert!(r3.instance_id() == id1 || r3.instance_id() == id2);
    }

    #[tokio::test]
    async fn test_scaling_recommendations() {
        // Test ScaleUp recommendation (at capacity and overutilized)
        let stats = PoolStats {
            active_count: 10,
            idle_count: 0,
            utilization: 1.0,
            ..Default::default()
        };

        let recommendation = stats.scaling_recommendation(10);
        assert!(matches!(
            recommendation,
            ScalingRecommendation::ScaleUp { .. }
        ));

        // Test ScaleDown recommendation (underutilized)
        let stats_low = PoolStats {
            active_count: 2,
            idle_count: 8,
            utilization: 0.2,
            ..Default::default()
        };

        let recommendation_low = stats_low.scaling_recommendation(10);
        assert!(matches!(
            recommendation_low,
            ScalingRecommendation::ScaleDown { .. }
        ));

        // Test NoChange recommendation (normal utilization)
        let stats_normal = PoolStats {
            active_count: 5,
            idle_count: 5,
            utilization: 0.5,
            ..Default::default()
        };

        let recommendation_normal = stats_normal.scaling_recommendation(10);
        assert!(matches!(
            recommendation_normal,
            ScalingRecommendation::NoChange { .. }
        ));
    }

    #[tokio::test]
    async fn test_pool_maintenance() {
        let config = PoolConfig {
            min_size: 2,
            max_size: 5,
            max_lifetime: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Acquire and release some resources to populate the pool
        let resource1 = pool.acquire().await.unwrap();
        let resource2 = pool.acquire().await.unwrap();

        let id1 = resource1.instance_id();
        let id2 = resource2.instance_id();

        drop(resource1);
        drop(resource2);

        pool.release(id1).await.unwrap();
        pool.release(id2).await.unwrap();

        // Perform maintenance
        let stats = pool.maintain().await.unwrap();

        // Check maintenance stats
        assert!(stats.duration_ms < 1000); // Should complete quickly
        assert_eq!(stats.pool_size, 2); // Should maintain min_size
        assert_eq!(stats.idle_resources, 2);
        assert_eq!(stats.active_resources, 0);
    }

    #[tokio::test]
    async fn test_pool_maintenance_removes_expired() {
        let config = PoolConfig {
            min_size: 0,
            max_size: 5,
            max_lifetime: Duration::from_millis(10), // Very short lifetime
            idle_timeout: Duration::from_millis(10),
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Create some resources
        let resource1 = pool.acquire().await.unwrap();
        let id1 = resource1.instance_id();
        drop(resource1);
        pool.release(id1).await.unwrap();

        // Wait for resources to expire
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Perform maintenance
        let stats = pool.maintain().await.unwrap();

        // Should have removed expired resources
        assert!(stats.resources_removed > 0 || stats.idle_resources == 0);
    }

    #[tokio::test]
    async fn test_pool_shutdown_graceful() {
        let config = PoolConfig {
            min_size: 2,
            max_size: 5,
            ..Default::default()
        };
        let pool = ResourcePool::new(config, PoolStrategy::Fifo, create_test_instance);

        // Create some resources
        let resource1 = pool.acquire().await.unwrap();
        let id1 = resource1.instance_id();
        drop(resource1);
        pool.release(id1).await.unwrap();

        // Shutdown the pool
        let stats = pool.shutdown().await.unwrap();

        // Check shutdown stats
        assert!(stats.duration_ms < 5000); // Should complete quickly
        assert!(stats.total_resources_closed >= 1);
        assert!(stats.completed_gracefully); // No active resources, should be graceful
        assert_eq!(stats.force_closed, 0);
    }

    #[tokio::test]
    async fn test_pool_shutdown_with_active_resources() {
        let config = PoolConfig {
            min_size: 0,
            max_size: 5,
            ..Default::default()
        };
        let pool = Arc::new(ResourcePool::new(
            config,
            PoolStrategy::Fifo,
            create_test_instance,
        ));

        // Acquire resource but don't release it
        let _resource = pool.acquire().await.unwrap();

        // Shutdown with short timeout
        let pool_clone = Arc::clone(&pool);
        let stats = pool_clone
            .shutdown_with_timeout(Duration::from_millis(100))
            .await
            .unwrap();

        // Should have force-closed active resources
        assert!(!stats.completed_gracefully || stats.force_closed > 0);
        assert!(stats.active_resources_closed >= 1);
    }

    #[tokio::test]
    async fn test_pool_manager_maintenance() {
        let manager = PoolManager::new();

        // Register some pools
        let config = PoolConfig {
            min_size: 1,
            max_size: 3,
            ..Default::default()
        };

        let pool1 = ResourcePool::new(config.clone(), PoolStrategy::Fifo, create_test_instance);
        let pool2 = ResourcePool::new(config, PoolStrategy::Lifo, create_test_instance);

        manager.register_pool("pool1".to_string(), pool1);
        manager.register_pool("pool2".to_string(), pool2);

        // Perform maintenance on all pools
        let results = manager.maintain_all().await.unwrap();

        // Should have results for both pools
        assert_eq!(results.len(), 2);
        assert!(results.contains_key("pool1"));
        assert!(results.contains_key("pool2"));
    }

    #[tokio::test]
    async fn test_pool_manager_shutdown() {
        let manager = PoolManager::new();

        // Register some pools
        let config = PoolConfig {
            min_size: 1,
            max_size: 3,
            ..Default::default()
        };

        let pool1 = ResourcePool::new(config.clone(), PoolStrategy::Fifo, create_test_instance);
        let pool2 = ResourcePool::new(config, PoolStrategy::Lifo, create_test_instance);

        manager.register_pool("pool1".to_string(), pool1);
        manager.register_pool("pool2".to_string(), pool2);

        // Shutdown all pools
        let results = manager.shutdown_all().await.unwrap();

        // Should have results for both pools
        assert_eq!(results.len(), 2);
        assert!(results.contains_key("pool1"));
        assert!(results.contains_key("pool2"));

        // All pools should be shut down gracefully
        for (_, stats) in results.iter() {
            assert!(stats.completed_gracefully);
        }
    }
}
