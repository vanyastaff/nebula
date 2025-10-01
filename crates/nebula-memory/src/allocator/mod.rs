//!
//! Custom allocators for memory management
//! This module provides various memory allocator implementations and management
//! utilities for different memory allocation patterns and requirements.
mod bump;
mod error;
mod manager;
#[cfg(all(feature = "std", feature = "monitoring"))]
mod monitored;
mod pool;
mod stack;
mod stats;
mod system;
mod tracked;
mod traits;
pub use bump::BumpAllocator;
pub use error::{
    AllocError, AllocErrorCode, AllocResult, ErrorStats, ErrorStatsSnapshot, MemoryState,
};
// Legacy types for backward compatibility
#[allow(deprecated)]
pub use error::AllocErrorKind;
pub use manager::{AllocatorId, AllocatorManager, GlobalAllocatorManager};
#[cfg(all(feature = "std", feature = "monitoring"))]
pub use monitored::{MonitoredAllocator, MonitoredConfig};
pub use pool::{PoolAllocator, PoolBox};
pub use stack::{StackAllocator, StackFrame, StackMarker};
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
