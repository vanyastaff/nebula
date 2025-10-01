//!
//! Custom allocators for memory management
//! This module provides various memory allocator implementations and management
//! utilities for different memory allocation patterns and requirements.
mod error;
mod manager;
#[cfg(all(feature = "std", feature = "monitoring"))]
mod monitored;
mod stats;
mod system;
mod tracked;
mod traits;
// Re-export from new modular structure
pub use crate::allocators::bump::BumpAllocator;
pub use crate::allocators::pool::{PoolAllocator, PoolBox, PoolStats};
pub use error::{
    AllocError, AllocErrorCode, AllocResult, ErrorStats, ErrorStatsSnapshot, MemoryState,
};
pub use manager::{AllocatorId, AllocatorManager, GlobalAllocatorManager};
#[cfg(all(feature = "std", feature = "monitoring"))]
pub use monitored::{MonitoredAllocator, MonitoredConfig};
pub use crate::allocators::stack::{StackAllocator, StackFrame, StackMarker, StackConfig};
pub use stats::{AllocatorStats, AtomicAllocatorStats, OptionalStats, StatisticsProvider};
pub use system::SystemAllocator;
pub use tracked::TrackedAllocator;
pub use traits::{
    Allocator, BasicMemoryUsage, BulkAllocator, MemoryUsage, Resettable, ThreadSafeAllocator,
};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn module_accessible() {
        let _manager = AllocatorManager::new();
    }
}
