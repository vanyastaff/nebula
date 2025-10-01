//! Pool allocator implementation
//!
//! A pool allocator for fixed-size blocks with lock-free free list.
//! Provides O(1) allocation/deallocation for same-sized objects.
//!
//! ## Status
//! ⚠️ PARTIAL MIGRATION - Main implementation still in allocator/pool.rs
//! This module currently contains extracted submodules. Full migration pending.

pub mod config;
pub mod pool_box;

pub use config::PoolConfig;
pub use pool_box::PoolBox;

// TODO: Extract main PoolAllocator implementation from allocator/pool.rs
// TODO: Extract block management internals
// TODO: Extract statistics types
