//! Custom allocators for memory management
//!
//! This module provides various memory allocator implementations and management
//! utilities for different memory allocation patterns and requirements.

mod bump;
mod error;
mod manager;
mod pool;
mod stack;
mod stats;
mod system;
mod tracked;
mod traits;

pub use bump::BumpAllocator;
pub use error::{AllocError, AllocErrorKind, AllocResult};
pub use manager::{AllocatorId, AllocatorManager, GlobalAllocatorManager};
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
