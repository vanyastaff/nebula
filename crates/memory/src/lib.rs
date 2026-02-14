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
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_memory::prelude::*;
//!
//! // Use a memory pool for object reuse
//! let mut pool = ObjectPool::new(10, || String::new());
//! let item = pool.get()?;
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
//! - `logging`: Integration with nebula-log
//! - `full`: Enable all features
//!
//! ## Architecture
//!
//! nebula-memory follows the Nebula ecosystem patterns:
//! - Standalone error handling via [`error`] module
//! - Optional structured logging via `nebula-log` (feature: `logging`)
//! - System integration via `nebula-system`
//! - Performance monitoring and metrics

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(clippy::all)]
#![warn(clippy::perf)]
#![warn(clippy::pedantic)]
#![warn(rust_2018_idioms)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]
// Bulk allows for doc lints — adding docs to 200+ functions is out of scope for this pass
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::too_many_lines)]
// Precision loss in usize/u64 -> f64 casts is acceptable for stats/metrics
#![allow(clippy::cast_precision_loss)]
// Explicit lifetimes are clearer in unsafe/arena code even when elidable
#![allow(clippy::elidable_lifetime_names)]
// Returning &str tied to &self is fine — these are accessor methods
#![allow(clippy::unnecessary_literal_bound)]
// inline(always) on small alignment/barrier helpers is intentional for hot paths
#![allow(clippy::inline_always)]
// Struct bool fields are configuration — splitting is over-engineering
#![allow(clippy::struct_excessive_bools)]
// Cast truncation/sign-loss in memory code is reviewed per-site
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
// #[must_use] on fns returning Self/Result documents intent even if type is already must_use
#![allow(clippy::double_must_use)]
#![allow(clippy::return_self_not_must_use)]
// Pointer alignment cast in pool allocator is intentional and safe
#![allow(clippy::cast_ptr_alignment)]
// Internal methods return Result for API consistency even when infallible today
#![allow(clippy::unnecessary_wraps)]

// Error types
pub mod error;

// Core modules
pub mod allocator;
#[cfg(feature = "arena")]
#[cfg_attr(docsrs, doc(cfg(feature = "arena")))]
pub mod arena;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod async_support;
#[cfg(feature = "budget")]
#[cfg_attr(docsrs, doc(cfg(feature = "budget")))]
pub mod budget;
#[cfg(feature = "cache")]
#[cfg_attr(docsrs, doc(cfg(feature = "cache")))]
pub mod cache;
pub mod core;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod extensions;
#[cfg(feature = "monitoring")]
#[cfg_attr(docsrs, doc(cfg(feature = "monitoring")))]
pub mod monitoring;
#[cfg(feature = "pool")]
#[cfg_attr(docsrs, doc(cfg(feature = "pool")))]
pub mod pool;
#[cfg(feature = "stats")]
#[cfg_attr(docsrs, doc(cfg(feature = "stats")))]
pub mod stats;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod syscalls;
pub mod utils;

// Re-export core types for convenience
pub use crate::core::MemoryConfig;
pub use crate::error::{MemoryError, MemoryResult, Result};

// Public API exports
pub mod prelude {
    //! Convenient re-exports of commonly used types and traits.

    // Core types
    pub use crate::core::MemoryConfig;
    pub use crate::core::traits::{MemoryManager, MemoryUsage, Resettable};

    // Error types (standalone!)
    pub use crate::error::{MemoryError, MemoryResult, Result};

    // Allocator types
    pub use crate::allocator::{
        AllocError, AllocResult, Allocator, GlobalAllocatorManager, TypedAllocator,
    };
    #[cfg(feature = "monitoring")]
    pub use crate::allocator::{MonitoredAllocator, MonitoredConfig};

    #[cfg(feature = "arena")]
    pub use crate::arena::{Arena, TypedArena};

    #[cfg(feature = "pool")]
    pub use crate::pool::{ObjectPool, PooledValue};

    #[cfg(feature = "cache")]
    pub use crate::cache::{CacheConfig, CacheKey, ComputeCache};

    #[cfg(feature = "budget")]
    pub use crate::budget::{BudgetConfig, BudgetMetrics, BudgetState, MemoryBudget};

    #[cfg(feature = "monitoring")]
    pub use crate::monitoring::{IntegratedStats, MemoryMonitor, MonitoringConfig, PressureAction};

    // Utility traits for safe arithmetic
    pub use crate::utils::CheckedArithmetic;
}

// Re-export allocator types at crate root for convenience
pub use crate::allocator::{AllocError, AllocResult};

#[cfg(feature = "logging")]
use nebula_log::{debug, info};

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
    crate::allocator::GlobalAllocatorManager::init().map_err(MemoryError::initialization_failed)?;

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
        info!("nebula-memory system shutdown complete");
    }

    Ok(())
}
