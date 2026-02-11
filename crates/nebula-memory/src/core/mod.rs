//! Core functionality for nebula-memory
//!
//! This module contains the fundamental building blocks of the memory management system:
//! - Error types and result handling
//! - Configuration structures
//! - Base traits for memory management
//! - Common types and constants
//! - Internal synchronization primitives

pub mod config;
pub(crate) mod sync_cell;
pub mod traits;
pub mod types;

// Re-export sync_cell for internal use
pub(crate) use sync_cell::SyncUnsafeCell;

// Re-export commonly used items
pub use crate::error::{MemoryError, MemoryResult};
pub use config::MemoryConfig;
pub use traits::{
    BasicMemoryUsage, CloneAllocator, MemoryManager, MemoryUsage, Resettable, StatisticsProvider,
};
pub use types::*;

/// Core prelude for convenient imports
pub mod prelude {
    pub use super::config::MemoryConfig;
    pub use super::traits::{MemoryManager, MemoryUsage};
    pub use super::types::*;
    pub use crate::error::{MemoryError, MemoryResult};
}
