//! # nebula-memory
//!
//! High-performance memory management for the Nebula workflow automation ecosystem.
//!
//! This crate provides zero-cost abstractions for memory management including:
//! - Custom allocators optimized for workflow execution
//! - Memory pools for object reuse
//! - Memory arenas for fast allocation/deallocation
//! - Multi-level caching systems
//! - Memory usage tracking and optimization
//! - Lock-free data structures for high concurrency
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_memory::prelude::*;
//!
//! // Use a memory pool for object reuse
//! let pool = ObjectPool::new();
//! let item = pool.acquire()?;
//! // item is automatically returned to pool when dropped
//!
//! // Use an arena for fast bulk allocation
//! let arena = Arena::new();
//! let data = arena.alloc_slice::<u64>(1000);
//! // entire arena is freed at once when dropped
//! ```
//!
//! ## Features
//!
//! - `std` (default): Enable standard library features
//! - `arena`: Memory arena allocators
//! - `pool`: Object pooling system
//! - `cache`: Multi-level caching
//! - `stats`: Memory usage statistics
//! - `budget`: Memory budget management
//! - `streaming`: Streaming data optimizations
//! - `logging`: Integration with nebula-log
//! - `full`: Enable all features
//!
//! ## Architecture
//!
//! nebula-memory follows the Nebula ecosystem patterns:
//! - Consistent error handling via [`nebula_error`]
//! - Structured logging via [`nebula_log`]
//! - System integration via [`nebula_system`]
//! - Performance monitoring and metrics

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Core error types and utilities
pub mod error;

// Memory allocators
pub mod allocator;

// Core traits for memory management
pub mod traits;

// Utility functions and helpers
pub mod utils;

// Core features that depend on allocators
#[cfg(feature = "arena")]
#[cfg_attr(docsrs, doc(cfg(feature = "arena")))]
pub mod arena;

#[cfg(feature = "pool")]
#[cfg_attr(docsrs, doc(cfg(feature = "pool")))]
pub mod pool;

#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;

// Advanced features
#[cfg(feature = "stats")]
#[cfg_attr(docsrs, doc(cfg(feature = "stats")))]
pub mod stats;

#[cfg(feature = "budget")]
#[cfg_attr(docsrs, doc(cfg(feature = "budget")))]
pub mod budget;

#[cfg(feature = "streaming")]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub mod streaming;

// Configuration and management
pub mod config;

// System integration
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod monitoring;

// Public API exports
pub mod prelude {
    //! Convenient re-exports of commonly used types and traits.

    pub use crate::allocator::{AllocError, AllocResult, Allocator, GlobalAllocatorManager};
    #[cfg(feature = "std")]
    pub use crate::allocator::{MonitoredAllocator, MonitoredConfig};
    pub use crate::error::{MemoryError, MemoryErrorCode, MemoryResult};
    pub use crate::traits::{MemoryManager, MemoryUsage};

    #[cfg(feature = "arena")]
    pub use crate::arena::{Arena, ArenaOptions, TypedArena};

    #[cfg(feature = "pool")]
    pub use crate::pool::{ObjectPool, PoolConfig, PooledObject};

    #[cfg(feature = "cache")]
    pub use crate::cache::{Cache, CacheConfig, CacheKey, CacheValue};

    #[cfg(feature = "stats")]
    pub use crate::stats::{MemoryStats, MemoryTracker, StatsCollector};

    #[cfg(feature = "budget")]
    pub use crate::budget::{MemoryBudget, BudgetConfig, BudgetTracker};

    #[cfg(feature = "std")]
    pub use crate::monitoring::{MemoryMonitor, MonitoringConfig, PressureAction, IntegratedStats};
}

// Re-export key types at crate root for convenience
pub use crate::error::{MemoryError, MemoryResult};
pub use crate::allocator::{AllocError, AllocResult};

#[cfg(feature = "logging")]
use nebula_log::{info, debug};

/// Initialize the nebula-memory system with default configuration.
///
/// This should be called once at application startup to set up
/// global memory management components.
///
/// # Examples
///
/// ```rust
/// use nebula_memory;
///
/// fn main() -> nebula_memory::MemoryResult<()> {
///     nebula_memory::init()?;
///
///     // Your application code here
///
///     Ok(())
/// }
/// ```
pub fn init() -> MemoryResult<()> {
    #[cfg(feature = "logging")]
    {
        debug!("Initializing nebula-memory system");
    }

    // Initialize global allocator manager
    crate::allocator::GlobalAllocatorManager::init()
        .map_err(|e| MemoryError::initialization_failed(e))?;

    #[cfg(feature = "stats")]
    {
        crate::stats::initialize_global_tracker()?;
    }

    #[cfg(feature = "logging")]
    {
        info!("nebula-memory system initialized successfully");
    }

    Ok(())
}

/// Shutdown the nebula-memory system and cleanup resources.
///
/// This should be called before application exit to ensure
/// proper cleanup of global resources.
pub fn shutdown() -> MemoryResult<()> {
    #[cfg(feature = "logging")]
    {
        debug!("Shutting down nebula-memory system");
    }

    #[cfg(feature = "stats")]
    {
        crate::stats::finalize_global_tracker()?;
    }

    #[cfg(feature = "logging")]
    {
        info!("nebula-memory system shutdown complete");
    }

    Ok(())
}