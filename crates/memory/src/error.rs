//! Standalone error types for nebula-memory
//!
//! Uses thiserror for clean, idiomatic Rust error definitions.

use core::alloc::Layout;
use thiserror::Error;

#[cfg(feature = "logging")]
use nebula_log::{error, warn};

// ============================================================================
// Main Error Types
// ============================================================================

/// Memory management errors
#[must_use = "errors should be handled"]
#[non_exhaustive]
#[derive(Error, Debug, Clone)]
pub enum MemoryError {
    // --- Allocation Errors ---
    #[error("Memory allocation failed: {size} bytes with {align} byte alignment")]
    AllocationFailed { size: usize, align: usize },

    #[error("Invalid memory layout: {reason}")]
    InvalidLayout { reason: String },

    #[error("Size overflow during operation: {operation}")]
    SizeOverflow { operation: String },

    #[error("Invalid alignment: {alignment}")]
    InvalidAlignment { alignment: usize },

    #[error("Allocation exceeds maximum size: {size} bytes (max: {max_size})")]
    ExceedsMaxSize { size: usize, max_size: usize },

    // --- Pool Errors ---
    #[error("Memory pool '{pool_id}' exhausted (capacity: {capacity})")]
    PoolExhausted { pool_id: String, capacity: usize },

    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },

    // --- Arena Errors ---
    #[error("Arena '{arena_id}' exhausted: requested {requested} bytes, available {available}")]
    ArenaExhausted {
        arena_id: String,
        requested: usize,
        available: usize,
    },

    // --- Cache Errors ---
    #[error("Cache miss for key: {key}")]
    CacheMiss { key: String },

    #[error("Cache overflow: {current} bytes used, {max} bytes maximum")]
    CacheOverflow { current: usize, max: usize },

    #[error("Invalid cache key: {reason}")]
    InvalidCacheKey { reason: String },

    // --- Budget Errors ---
    #[error("Memory budget exceeded: {used} bytes used, {limit} bytes limit")]
    BudgetExceeded { used: usize, limit: usize },

    // --- System Errors ---
    #[error("Memory corruption detected in {component}: {details}")]
    Corruption { component: String, details: String },

    #[error("Concurrent access error: {details}")]
    ConcurrentAccess { details: String },

    #[error("Invalid state: {reason}")]
    InvalidState { reason: String },

    #[error("Initialization failed: {reason}")]
    InitializationFailed { reason: String },

    // --- Feature Support Errors ---
    #[error("Feature not supported: {feature}{}", context.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    NotSupported {
        feature: &'static str,
        context: Option<String>,
    },

    // --- General Errors ---
    #[error("Operation not found: {reason}")]
    NotFound { reason: String },

    #[error("Invalid operation: {reason}")]
    InvalidOperation { reason: String },
}

impl MemoryError {
    /// Check if error is retryable
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::PoolExhausted { .. }
                | Self::ArenaExhausted { .. }
                | Self::CacheOverflow { .. }
                | Self::BudgetExceeded { .. }
                | Self::CacheMiss { .. }
        )
    }

    /// Get error code for categorization
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::AllocationFailed { .. } => "MEM:ALLOC:FAILED",
            Self::InvalidLayout { .. } => "MEM:ALLOC:LAYOUT",
            Self::SizeOverflow { .. } => "MEM:ALLOC:OVERFLOW",
            Self::InvalidAlignment { .. } => "MEM:ALLOC:ALIGN",
            Self::ExceedsMaxSize { .. } => "MEM:ALLOC:MAX",
            Self::PoolExhausted { .. } => "MEM:POOL:EXHAUSTED",
            Self::InvalidConfig { .. } => "MEM:CONFIG:INVALID",
            Self::ArenaExhausted { .. } => "MEM:ARENA:EXHAUSTED",
            Self::CacheMiss { .. } => "MEM:CACHE:MISS",
            Self::CacheOverflow { .. } => "MEM:CACHE:OVERFLOW",
            Self::InvalidCacheKey { .. } => "MEM:CACHE:KEY",
            Self::BudgetExceeded { .. } => "MEM:BUDGET:EXCEEDED",
            Self::Corruption { .. } => "MEM:SYSTEM:CORRUPTION",
            Self::ConcurrentAccess { .. } => "MEM:SYSTEM:CONCURRENT",
            Self::InvalidState { .. } => "MEM:SYSTEM:STATE",
            Self::InitializationFailed { .. } => "MEM:SYSTEM:INIT",
            Self::NotSupported { .. } => "MEM:FEATURE:UNSUPPORTED",
            Self::NotFound { .. } => "MEM:NOT_FOUND",
            Self::InvalidOperation { .. } => "MEM:INVALID_OP",
        }
    }

    // ============================================================================
    // Convenience Constructors - Allocation Errors
    // ============================================================================

    /// Create allocation failed error
    pub fn allocation_failed(size: usize, align: usize) -> Self {
        #[cfg(feature = "logging")]
        error!(
            "Memory allocation failed: {} bytes with {} alignment",
            size, align
        );

        Self::AllocationFailed { size, align }
    }

    /// Create allocation failed error from layout
    #[must_use]
    pub fn allocation_failed_with_layout(layout: Layout) -> Self {
        Self::allocation_failed(layout.size(), layout.align())
    }

    /// Create invalid layout error
    pub fn invalid_layout(reason: &str) -> Self {
        Self::InvalidLayout {
            reason: reason.to_string(),
        }
    }

    /// Create size overflow error
    pub fn size_overflow(operation: &str) -> Self {
        Self::SizeOverflow {
            operation: operation.to_string(),
        }
    }

    /// Create invalid alignment error
    #[must_use]
    pub fn invalid_alignment(alignment: usize) -> Self {
        Self::InvalidAlignment { alignment }
    }

    /// Create allocation too large error
    #[must_use]
    pub fn allocation_too_large(size: usize, max_size: usize) -> Self {
        Self::ExceedsMaxSize { size, max_size }
    }

    // --- Pool Errors ---

    /// Create pool exhausted error
    pub fn pool_exhausted(pool_id: &str, capacity: usize) -> Self {
        #[cfg(feature = "logging")]
        warn!("Memory pool exhausted: {}", pool_id);

        Self::PoolExhausted {
            pool_id: pool_id.to_string(),
            capacity,
        }
    }

    /// Create invalid pool config error
    pub fn invalid_pool_config(reason: &str) -> Self {
        Self::InvalidConfig {
            reason: format!("invalid pool config: {reason}"),
        }
    }

    /// Create invalid config error
    pub fn invalid_config(reason: &str) -> Self {
        Self::InvalidConfig {
            reason: reason.to_string(),
        }
    }

    // --- Arena Errors ---

    /// Create arena exhausted error
    pub fn arena_exhausted(arena_id: &str, requested: usize, available: usize) -> Self {
        Self::ArenaExhausted {
            arena_id: arena_id.to_string(),
            requested,
            available,
        }
    }

    /// Create invalid arena operation error
    pub fn invalid_arena_operation(operation: &str) -> Self {
        Self::InvalidState {
            reason: format!("invalid arena operation: {operation}"),
        }
    }

    // --- Cache Errors ---

    /// Create cache miss error
    pub fn cache_miss(key: &str) -> Self {
        Self::CacheMiss {
            key: key.to_string(),
        }
    }

    /// Create cache full error
    #[must_use]
    pub fn cache_full(capacity: usize) -> Self {
        Self::CacheOverflow {
            current: capacity,
            max: capacity,
        }
    }

    /// Create invalid cache key error
    pub fn invalid_cache_key(key: &str) -> Self {
        Self::InvalidCacheKey {
            reason: format!("invalid key: {key}"),
        }
    }

    // --- Budget Errors ---

    /// Create budget exceeded error
    #[must_use]
    pub fn budget_exceeded(used: usize, limit: usize) -> Self {
        Self::BudgetExceeded { used, limit }
    }

    /// Create invalid budget error
    pub fn invalid_budget(reason: &str) -> Self {
        Self::InvalidConfig {
            reason: format!("invalid budget: {reason}"),
        }
    }

    // --- System Errors ---

    /// Create memory corruption error
    pub fn corruption(component: &str, details: &str) -> Self {
        #[cfg(feature = "logging")]
        error!("Memory corruption: {component} - {details}");

        Self::Corruption {
            component: component.to_string(),
            details: details.to_string(),
        }
    }

    /// Create concurrent access error
    pub fn concurrent_access(details: &str) -> Self {
        Self::ConcurrentAccess {
            details: details.to_string(),
        }
    }

    /// Create leak detected error
    pub fn leak_detected(size: usize, location: &str) -> Self {
        Self::Corruption {
            component: "memory tracker".to_string(),
            details: format!("leak detected: {size} bytes at {location}"),
        }
    }

    /// Create fragmentation error
    #[must_use]
    pub fn fragmentation(available: usize, largest_block: usize, requested: usize) -> Self {
        Self::InvalidState {
            reason: format!(
                "fragmentation: {available} bytes available, largest block {largest_block}, requested {requested}"
            ),
        }
    }

    /// Create initialization failed error
    pub fn initialization_failed(component: &str) -> Self {
        Self::InitializationFailed {
            reason: format!("failed to initialize {component}"),
        }
    }

    /// Create invalid input error
    pub fn invalid_input(reason: &str) -> Self {
        Self::invalid_layout(reason)
    }

    /// Create invalid argument error (alias for `invalid_input`)
    pub fn invalid_argument(reason: &str) -> Self {
        Self::invalid_input(reason)
    }

    /// Create invalid index error
    #[must_use]
    pub fn invalid_index(index: usize, max: usize) -> Self {
        Self::InvalidState {
            reason: format!("invalid index {index} (max: {max})"),
        }
    }

    /// Create decompression failed error
    pub fn decompression_failed(reason: &str) -> Self {
        Self::InvalidState {
            reason: format!("decompression failed: {reason}"),
        }
    }

    /// Create monitor error
    pub fn monitor_error(reason: &str) -> Self {
        Self::InvalidState {
            reason: format!("monitor error: {reason}"),
        }
    }

    /// Create not supported error
    pub fn not_supported(operation: &str) -> Self {
        Self::InvalidState {
            reason: format!("operation not supported: {operation}"),
        }
    }

    /// Create out of memory error
    #[must_use]
    pub fn out_of_memory(size: usize, align: usize) -> Self {
        Self::allocation_failed(size, align)
    }

    /// Create out of memory error with layout
    #[must_use]
    pub fn out_of_memory_with_layout(layout: Layout) -> Self {
        Self::allocation_failed_with_layout(layout)
    }

    /// Check if this is an invalid alignment error
    #[must_use]
    pub fn is_invalid_alignment(&self) -> bool {
        matches!(self, Self::InvalidAlignment { .. })
    }
}

// ============================================================================
// Result Types
// ============================================================================

/// Result type for memory operations
pub type MemoryResult<T> = core::result::Result<T, MemoryError>;

/// Generic result type alias
pub type Result<T> = MemoryResult<T>;

/// Type aliases for allocator module backward compatibility
pub type AllocError = MemoryError;
pub type AllocResult<T> = MemoryResult<T>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_error_creation() {
        let error = MemoryError::allocation_failed(1024, 8);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("1024"));
    }

    #[test]
    fn test_error_with_layout() {
        let layout = Layout::new::<u64>();
        let error = MemoryError::allocation_failed_with_layout(layout);
        assert!(error.to_string().contains(&layout.size().to_string()));
    }

    #[test]
    fn test_convenience_constructors() {
        let alloc_error = MemoryError::allocation_failed(1024, 8);
        let pool_error = MemoryError::pool_exhausted("test_pool", 100);
        let cache_error = MemoryError::cache_miss("test_key");

        assert!(!alloc_error.to_string().is_empty());
        assert!(!pool_error.to_string().is_empty());
        assert!(!cache_error.to_string().is_empty());
    }

    #[test]
    fn test_arena_errors() {
        let error = MemoryError::arena_exhausted("test_arena", 1024, 512);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("test_arena"));
    }

    #[test]
    fn test_budget_errors() {
        let error = MemoryError::budget_exceeded(2048, 1024);
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("2048"));
    }

    #[test]
    fn test_corruption_errors() {
        let error = MemoryError::corruption("allocator", "invalid pointer");
        assert!(!error.to_string().is_empty());
        assert!(error.to_string().contains("allocator"));
    }

    #[test]
    fn test_error_codes() {
        let error = MemoryError::allocation_failed(1024, 8);
        assert_eq!(error.code(), "MEM:ALLOC:FAILED");

        let error = MemoryError::pool_exhausted("test", 100);
        assert_eq!(error.code(), "MEM:POOL:EXHAUSTED");
    }

    #[test]
    fn test_retryable() {
        assert!(MemoryError::pool_exhausted("test", 100).is_retryable());
        assert!(!MemoryError::invalid_alignment(8).is_retryable());
    }
}
