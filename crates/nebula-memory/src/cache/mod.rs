//! Cache implementations for memory management
//!
//! This module provides various caching mechanisms to optimize memory usage
//! and improve performance by reusing computed values.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Core cache types
#[cfg(feature = "async")]
mod async_compute;
mod compute;
#[cfg(feature = "std")]
pub mod concurrent;
mod config;
mod multi_level;
mod partitioned;
pub mod policies;
mod scheduled;
#[cfg(all(feature = "std", feature = "async"))]
pub mod simple;
mod stats;

// Re-exports for convenience
pub use compute::{CacheEntry, CacheKey, CacheResult, ComputeCache};
#[cfg(feature = "std")]
pub use concurrent::ConcurrentComputeCache;
pub use config::{CacheConfig, CacheMetrics, EvictionPolicy};
pub use multi_level::{CacheLevel, MultiLevelCache, MultiLevelStats, PromotionPolicy};
pub use partitioned::PartitionedCache;
pub use scheduled::{ScheduledCache, ScheduledTask};
#[cfg(all(feature = "std", feature = "async"))]
pub use simple::{AsyncCache, CacheStats as SimpleCacheStats};
pub use stats::{AtomicCacheStats, CacheStats, StatsCollector, StatsProvider};

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

