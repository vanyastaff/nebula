//! Cache implementations for memory management
//!
//! This module provides various caching mechanisms to optimize memory usage
//! and improve performance by reusing computed values.

// Core cache types
mod compute;
pub mod concurrent;
mod config;
pub mod multi_level;
pub mod partitioned;
pub mod policies;
pub mod scheduled;
#[cfg(feature = "async")]
pub mod simple;
pub mod stats;
// Re-exports for convenience
pub use compute::{CacheEntry, CacheKey, CacheResult, ComputeCache};
pub use concurrent::ConcurrentComputeCache;
pub use config::{CacheConfig, CacheMetrics, EvictionPolicy};
pub use multi_level::MultiLevelCache;
pub use partitioned::PartitionedCache;
pub use scheduled::ScheduledCache;
#[cfg(feature = "async")]
pub use simple::{AsyncCache, CacheStats as SimpleCacheStats};
pub use stats::{AtomicCacheStats, CacheStats, StatsCollector, StatsProvider};

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn module_accessible() {
        let _config = CacheConfig::default();
    }
}
