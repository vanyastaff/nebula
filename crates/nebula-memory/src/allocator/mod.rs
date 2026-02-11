//!
//! Custom allocators for memory management
//! This module provides various memory allocator implementations and management
//! utilities for different memory allocation patterns and requirements.

// Core allocator types
mod manager;
#[cfg(feature = "monitoring")]
mod monitored;
mod stats;
mod system;
mod tracked;
mod traits;

// Allocator implementations
pub mod bump;
pub mod pool;
pub mod stack;

// Re-exports for convenience
pub use bump::BumpAllocator;
#[cfg(feature = "stats")]
pub use pool::PoolStats;
pub use pool::{PoolAllocator, PoolBox, PoolConfig};

pub use crate::error::{AllocError, AllocResult};
pub use manager::{AllocatorId, AllocatorManager, GlobalAllocatorManager};
#[cfg(feature = "monitoring")]
pub use monitored::{MonitoredAllocator, MonitoredConfig};
pub use stack::{StackAllocator, StackConfig, StackFrame, StackMarker};
pub use stats::BatchedStats;
pub use stats::{AllocatorStats, AtomicAllocatorStats, OptionalStats, StatisticsProvider};
pub use system::SystemAllocator;
pub use tracked::TrackedAllocator;
pub use traits::{
    Allocator, BasicMemoryUsage, BulkAllocator, MemoryUsage, Resettable, ThreadSafeAllocator,
    TypedAllocator,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn module_accessible() {
        let _manager = AllocatorManager::new();
    }
}
