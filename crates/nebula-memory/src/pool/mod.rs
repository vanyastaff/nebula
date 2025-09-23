//! Object pooling for efficient memory reuse
//!
//! This module provides various object pool implementations for different use
//! cases:
//! - `ObjectPool`: Basic single-threaded pool
//! - `ThreadSafePool`: Multi-threaded pool with mutex
//! - `LockFreePool`: Lock-free pool for high concurrency
//! - `PriorityPool`: Pool with priority-based retention
//! - `TtlPool`: Pool with time-to-live for objects
//! - `HierarchicalPool`: Multi-level pool hierarchy
//! - `BatchAllocator`: Batch allocation optimization

mod batch;
mod hierarchical;
mod lockfree;
mod object_pool;
mod poolable;
mod priority;
mod stats;
mod thread_safe;
mod ttl;

#[cfg(feature = "std")]
use std::time::Duration;

pub use batch::BatchAllocator;
pub use hierarchical::HierarchicalPool;
pub use lockfree::LockFreePool;
pub use object_pool::{ObjectPool, PooledValue};
pub use poolable::Poolable;
pub use priority::PriorityPool;
pub use stats::PoolStats;
pub use thread_safe::ThreadSafePool;
pub use ttl::TtlPool;

/// Configuration for object pools
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Initial capacity of the pool
    pub initial_capacity: usize,

    /// Maximum capacity (None for unbounded)
    pub max_capacity: Option<usize>,

    /// Enable statistics collection
    #[cfg(feature = "stats")]
    pub track_stats: bool,

    /// Validate objects on return
    pub validate_on_return: bool,

    /// Pre-warm pool on creation
    pub pre_warm: bool,

    /// Time-to-live for pooled objects
    #[cfg(feature = "std")]
    pub ttl: Option<Duration>,

    /// Growth strategy when pool is empty
    pub growth_strategy: GrowthStrategy,

    /// Memory pressure threshold (percentage)
    #[cfg(feature = "adaptive")]
    pub pressure_threshold: u8,
}

/// Growth strategy for pools
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthStrategy {
    /// Fixed increment
    Fixed(usize),
    /// Percentage growth
    Percentage(u8),
    /// Double the size
    Double,
    /// No growth (bounded pool)
    None,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            initial_capacity: 128,
            max_capacity: None,
            #[cfg(feature = "stats")]
            track_stats: true,
            #[cfg(debug_assertions)]
            validate_on_return: true,
            #[cfg(not(debug_assertions))]
            validate_on_return: false,
            pre_warm: true,
            #[cfg(feature = "std")]
            ttl: None,
            growth_strategy: GrowthStrategy::Double,
            #[cfg(feature = "adaptive")]
            pressure_threshold: 75,
        }
    }
}

impl PoolConfig {
    /// Create a bounded pool configuration
    pub fn bounded(capacity: usize) -> Self {
        Self {
            initial_capacity: capacity,
            max_capacity: Some(capacity),
            growth_strategy: GrowthStrategy::None,
            ..Default::default()
        }
    }

    /// Create an unbounded pool configuration
    pub fn unbounded(initial_capacity: usize) -> Self {
        Self { initial_capacity, max_capacity: None, ..Default::default() }
    }

    /// Set time-to-live for objects
    #[cfg(feature = "std")]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Set growth strategy
    pub fn with_growth_strategy(mut self, strategy: GrowthStrategy) -> Self {
        self.growth_strategy = strategy;
        self
    }

    /// Enable or disable validation on return
    pub fn with_validation(mut self, validate: bool) -> Self {
        self.validate_on_return = validate;
        self
    }

    /// Enable or disable pre-warming
    pub fn with_pre_warm(mut self, pre_warm: bool) -> Self {
        self.pre_warm = pre_warm;
        self
    }

    #[cfg(feature = "stats")]
    /// Enable or disable statistics tracking
    pub fn with_stats(mut self, track_stats: bool) -> Self {
        self.track_stats = track_stats;
        self
    }

    /// Set maximum capacity
    pub fn with_max_capacity(mut self, max_capacity: Option<usize>) -> Self {
        self.max_capacity = max_capacity;
        self
    }

    #[cfg(feature = "adaptive")]
    /// Set memory pressure threshold
    pub fn with_pressure_threshold(mut self, threshold: u8) -> Self {
        assert!(threshold <= 100, "Pressure threshold must be between 0 and 100");
        self.pressure_threshold = threshold;
        self
    }
}

/// Pool lifecycle callbacks
pub trait PoolCallbacks<T>: Send + Sync {
    /// Called when object is created
    fn on_create(&self, _obj: &T) {}

    /// Called when object is checked out
    fn on_checkout(&self, _obj: &T) {}

    /// Called when object is returned
    fn on_checkin(&self, _obj: &T) {}

    /// Called when object is destroyed
    fn on_destroy(&self, _obj: &T) {}
}

/// Default no-op callbacks
pub struct NoOpCallbacks;

impl<T> PoolCallbacks<T> for NoOpCallbacks {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_bounded() {
        let config = PoolConfig::bounded(100);
        assert_eq!(config.initial_capacity, 100);
        assert_eq!(config.max_capacity, Some(100));
        assert_eq!(config.growth_strategy, GrowthStrategy::None);
    }

    #[test]
    fn test_pool_config_unbounded() {
        let config = PoolConfig::unbounded(50);
        assert_eq!(config.initial_capacity, 50);
        assert_eq!(config.max_capacity, None);
        assert_eq!(config.growth_strategy, GrowthStrategy::Double);
    }
}
