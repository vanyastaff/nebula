//! Cache implementations for memory management
//!
//! This module provides various caching mechanisms to optimize memory usage
//! and improve performance by reusing computed values.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Core cache types
mod compute;
#[cfg(feature = "std")]
pub mod concurrent;
mod config;
pub mod policies;
#[cfg(feature = "std")]
pub mod scheduled;
#[cfg(all(feature = "std", feature = "async"))]
pub mod simple;
pub mod stats;
// Re-exports for convenience
pub use compute::{CacheEntry, CacheKey, CacheResult, ComputeCache};
#[cfg(feature = "std")]
pub use concurrent::ConcurrentComputeCache;
pub use config::{CacheConfig, CacheMetrics, EvictionPolicy};
#[cfg(feature = "std")]
pub use scheduled::ScheduledCache;
#[cfg(all(feature = "std", feature = "async"))]
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










