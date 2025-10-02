//! Core functionality for nebula-memory
//!
//! This module contains the fundamental building blocks of the memory management system:
//! - Error types and result handling
//! - Configuration structures
//! - Base traits for memory management
//! - Common types and constants

pub mod error;
pub mod config;
pub mod traits;
pub mod types;

// Re-export commonly used items
pub use error::{MemoryError, MemoryErrorCode, MemoryResult};
pub use config::MemoryConfig;
pub use traits::{
    MemoryManager, MemoryUsage, Resettable, BasicMemoryUsage,
    CloneAllocator, StatisticsProvider
};
pub use types::*;

/// Core prelude for convenient imports
pub mod prelude {
    pub use super::error::{MemoryError, MemoryErrorCode, MemoryResult};
    pub use super::config::MemoryConfig;
    pub use super::traits::{MemoryManager, MemoryUsage};
    pub use super::types::*;
}
