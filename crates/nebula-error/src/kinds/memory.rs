//! Memory management error types
//!
//! This module provides comprehensive error types for memory management operations
//! including allocation, pooling, arenas, caching, and budget management.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::traits::ErrorCode;
use crate::kinds::codes;

/// Memory management errors
///
/// Covers all memory-related error scenarios including allocation failures,
/// pool exhaustion, arena management, caching, and budget enforcement.
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum MemoryError {
    /// Memory allocation failed due to insufficient memory
    #[error("Memory allocation failed: {size} bytes with {align} byte alignment")]
    AllocationFailed {
        /// Requested size in bytes
        size: usize,
        /// Required alignment in bytes
        align: usize,
    },

    /// Memory pool is exhausted and cannot provide more objects
    #[error("Memory pool exhausted: {pool_id} (capacity: {capacity})")]
    PoolExhausted {
        /// Identifier of the exhausted pool
        pool_id: String,
        /// Maximum capacity of the pool
        capacity: usize,
    },

    /// Memory arena has insufficient space for the allocation
    #[error("Memory arena exhausted: {arena_id} (requested: {requested}, available: {available})")]
    ArenaExhausted {
        /// Identifier of the exhausted arena
        arena_id: String,
        /// Requested allocation size
        requested: usize,
        /// Available space in the arena
        available: usize,
    },

    /// Cache lookup resulted in a miss (key not found)
    #[error("Cache miss: key '{key}'")]
    CacheMiss {
        /// The cache key that was not found
        key: String,
    },

    /// Memory budget limit has been exceeded
    #[error("Memory budget exceeded: used {used} bytes, limit {limit} bytes")]
    BudgetExceeded {
        /// Current memory usage in bytes
        used: usize,
        /// Maximum allowed memory in bytes
        limit: usize,
    },

    /// Invalid memory layout parameters provided
    #[error("Invalid memory layout: {reason}")]
    InvalidLayout {
        /// Reason why the layout is invalid
        reason: String,
    },

    /// Memory corruption detected in internal structures
    #[error("Memory corruption detected in {component}: {details}")]
    Corruption {
        /// Component where corruption was detected
        component: String,
        /// Details about the corruption
        details: String,
    },

    /// Invalid memory configuration parameters
    #[error("Invalid memory configuration: {reason}")]
    InvalidConfig {
        /// Reason why the configuration is invalid
        reason: String,
    },

    /// Memory system initialization failed
    #[error("Memory system initialization failed: {reason}")]
    InitializationFailed {
        /// Reason for initialization failure
        reason: String,
    },

    /// Invalid alignment value (must be power of two)
    #[error("Invalid alignment: {alignment} (must be power of two)")]
    InvalidAlignment {
        /// The invalid alignment value
        alignment: usize,
    },

    /// Size calculation overflow
    #[error("Memory size calculation overflow: {operation}")]
    SizeOverflow {
        /// Operation that caused the overflow
        operation: String,
    },

    /// Allocation size exceeds maximum supported size
    #[error("Allocation size {size} exceeds maximum {max_size}")]
    ExceedsMaxSize {
        /// Requested size
        size: usize,
        /// Maximum supported size
        max_size: usize,
    },

    /// Cache capacity overflow
    #[error("Cache capacity exceeded: {current}/{max}")]
    CacheOverflow {
        /// Current cache size
        current: usize,
        /// Maximum cache size
        max: usize,
    },

    /// Invalid cache key provided
    #[error("Invalid cache key: {reason}")]
    InvalidCacheKey {
        /// Reason why the key is invalid
        reason: String,
    },

    /// Invalid memory state detected
    #[error("Invalid memory state: {reason}")]
    InvalidState {
        /// Description of the invalid state
        reason: String,
    },

    /// Concurrent access violation
    #[error("Concurrent access violation: {details}")]
    ConcurrentAccess {
        /// Details about the concurrent access
        details: String,
    },

    /// Resource limit exceeded
    #[error("Resource limit exceeded: {resource}")]
    ResourceLimit {
        /// Resource that exceeded its limit
        resource: String,
    },
}

impl ErrorCode for MemoryError {
    fn error_code(&self) -> &str {
        match self {
            MemoryError::AllocationFailed { .. } => codes::MEMORY_ALLOCATION_FAILED,
            MemoryError::PoolExhausted { .. } => codes::MEMORY_POOL_EXHAUSTED,
            MemoryError::ArenaExhausted { .. } => codes::MEMORY_ARENA_EXHAUSTED,
            MemoryError::CacheMiss { .. } => codes::MEMORY_CACHE_MISS,
            MemoryError::BudgetExceeded { .. } => codes::MEMORY_BUDGET_EXCEEDED,
            MemoryError::InvalidLayout { .. } => codes::MEMORY_INVALID_LAYOUT,
            MemoryError::Corruption { .. } => codes::MEMORY_CORRUPTION,
            MemoryError::InvalidConfig { .. } => codes::MEMORY_INVALID_CONFIG,
            MemoryError::InitializationFailed { .. } => codes::MEMORY_INITIALIZATION_FAILED,
            MemoryError::InvalidAlignment { .. } => codes::MEMORY_INVALID_ALIGNMENT,
            MemoryError::SizeOverflow { .. } => codes::MEMORY_SIZE_OVERFLOW,
            MemoryError::ExceedsMaxSize { .. } => codes::MEMORY_EXCEEDS_MAX_SIZE,
            MemoryError::CacheOverflow { .. } => codes::MEMORY_CACHE_OVERFLOW,
            MemoryError::InvalidCacheKey { .. } => codes::MEMORY_INVALID_CACHE_KEY,
            MemoryError::InvalidState { .. } => codes::MEMORY_INVALID_STATE,
            MemoryError::ConcurrentAccess { .. } => codes::MEMORY_CONCURRENT_ACCESS,
            MemoryError::ResourceLimit { .. } => codes::MEMORY_RESOURCE_LIMIT,
        }
    }

    fn error_category(&self) -> &'static str {
        codes::CATEGORY_MEMORY
    }
}

impl MemoryError {
    /// Check if this error is retryable
    ///
    /// Memory allocation failures and resource exhaustion are typically retryable
    /// as the situation may improve over time (e.g., memory freed by GC, cache eviction).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MemoryError::AllocationFailed { .. }
                | MemoryError::PoolExhausted { .. }
                | MemoryError::ArenaExhausted { .. }
                | MemoryError::BudgetExceeded { .. }
                | MemoryError::CacheOverflow { .. }
                | MemoryError::ResourceLimit { .. }
        )
    }

    /// Check if this is a critical error that requires immediate attention
    #[must_use]
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            MemoryError::Corruption { .. }
                | MemoryError::ConcurrentAccess { .. }
                | MemoryError::AllocationFailed { .. }
        )
    }

    /// Check if this is a configuration error
    #[must_use]
    pub fn is_config_error(&self) -> bool {
        matches!(
            self,
            MemoryError::InvalidConfig { .. }
                | MemoryError::InvalidLayout { .. }
                | MemoryError::InvalidAlignment { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_failed() {
        let err = MemoryError::AllocationFailed {
            size: 1024,
            align: 8,
        };
        assert_eq!(err.error_code(), codes::MEMORY_ALLOCATION_FAILED);
        assert_eq!(err.error_category(), codes::CATEGORY_MEMORY);
        assert!(err.is_retryable());
        assert!(err.is_critical());
        assert!(!err.is_config_error());
    }

    #[test]
    fn test_pool_exhausted() {
        let err = MemoryError::PoolExhausted {
            pool_id: "worker_pool".to_string(),
            capacity: 100,
        };
        assert_eq!(err.error_code(), codes::MEMORY_POOL_EXHAUSTED);
        assert!(err.is_retryable());
        assert!(!err.is_critical());
    }

    #[test]
    fn test_cache_miss() {
        let err = MemoryError::CacheMiss {
            key: "user:123".to_string(),
        };
        assert_eq!(err.error_code(), codes::MEMORY_CACHE_MISS);
        assert!(!err.is_retryable());
        assert!(!err.is_critical());
    }

    #[test]
    fn test_corruption() {
        let err = MemoryError::Corruption {
            component: "bump_allocator".to_string(),
            details: "invalid free list pointer".to_string(),
        };
        assert_eq!(err.error_code(), codes::MEMORY_CORRUPTION);
        assert!(err.is_critical());
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_invalid_config() {
        let err = MemoryError::InvalidConfig {
            reason: "pool size must be positive".to_string(),
        };
        assert_eq!(err.error_code(), codes::MEMORY_INVALID_CONFIG);
        assert!(err.is_config_error());
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_error_display() {
        let err = MemoryError::BudgetExceeded {
            used: 1000,
            limit: 500,
        };
        let display = format!("{}", err);
        assert!(display.contains("1000"));
        assert!(display.contains("500"));
    }
}
