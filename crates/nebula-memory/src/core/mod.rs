//! Core functionality for nebula-memory
//!
//! This module contains the fundamental building blocks of the memory management system:
//! - Error types and result handling
//! - Configuration structures
//! - Base traits for memory management
//! - Common types and constants

pub mod config;
pub mod traits;
pub mod types;

// Re-export commonly used items
pub use config::MemoryConfig;
pub use crate::error::{MemoryError, MemoryResult};
pub use traits::{
    BasicMemoryUsage, CloneAllocator, MemoryManager, MemoryUsage, Resettable, StatisticsProvider,
};
pub use types::*;

/// Core prelude for convenient imports
pub mod prelude {
    pub use super::config::MemoryConfig;
    pub use crate::error::{MemoryError, MemoryResult};
    pub use super::traits::{MemoryManager, MemoryUsage};
    pub use super::types::*;
}

