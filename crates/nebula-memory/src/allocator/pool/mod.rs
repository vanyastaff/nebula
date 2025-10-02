//! Pool allocator implementation
//!
//! A pool allocator for fixed-size blocks with lock-free free list.
//! Provides O(1) allocation/deallocation for same-sized objects.
//!
//! ## Modules
//! - `allocator` - Main PoolAllocator implementation with lock-free free list
//! - `config` - Configuration variants (production, debug, performance)
//! - `pool_box` - RAII smart pointer for pool-allocated objects
//! - `stats` - Statistics tracking types

pub mod allocator;
pub mod config;
pub mod pool_box;
pub mod stats;

pub use allocator::PoolAllocator;
pub use config::PoolConfig;
pub use pool_box::PoolBox;
pub use stats::PoolStats;

