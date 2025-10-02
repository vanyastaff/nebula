//! Cache implementations for memory management
//!
//! This module provides various caching mechanisms to optimize memory usage
//! and improve performance by reusing computed values.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Core cache types
mod compute;
mod config;
mod stats;

// Advanced cache implementations
mod multi_level;
mod partitioned;
mod scheduled;

// Cache eviction policies
pub mod policies;

// Re-exports for convenience
pub use compute::{CacheEntry, CacheKey, CacheResult, ComputeCache};
pub use config::{CacheConfig, CacheMetrics, EvictionPolicy};
pub use multi_level::{CacheLevel, MultiLevelCache, MultiLevelStats, PromotionPolicy};
pub use partitioned::PartitionedCache;
pub use scheduled::{ScheduledCache, ScheduledTask};
pub use stats::{AtomicCacheStats, CacheStats, StatsCollector, StatsProvider};

#[cfg(feature = "async")]
mod async_compute;

#[cfg(feature = "async")]
pub use async_compute::{AsyncCacheResult, AsyncComputeCache};
#[cfg(feature = "std")]
pub use scheduled::ExpiredEntriesCleanupTask;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_accessible() {
        let _config = CacheConfig::default();
    }
}
