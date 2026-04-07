//! Standalone error types for nebula-memory
//!
//! Uses thiserror for clean, idiomatic Rust error definitions.

use core::alloc::Layout;
use thiserror::Error;

// ============================================================================
// Main Error Types
// ============================================================================

/// Memory management errors
#[must_use = "errors should be handled"]
#[non_exhaustive]
#[derive(Error, Debug, Clone, nebula_error::Classify)]
pub enum MemoryError {
    // --- Allocation Errors ---
    #[classify(category = "internal", code = "MEM:ALLOC:FAILED")]
    #[error("Memory allocation failed: {size} bytes with {align} byte alignment")]
    AllocationFailed { size: usize, align: usize },

    #[classify(category = "internal", code = "MEM:ALLOC:LAYOUT")]
    #[error("Invalid memory layout: {reason}")]
    InvalidLayout { reason: Box<str> },

    #[classify(category = "internal", code = "MEM:ALLOC:OVERFLOW")]
    #[error("Size overflow during operation: {operation}")]
    SizeOverflow { operation: Box<str> },

    #[classify(category = "internal", code = "MEM:ALLOC:ALIGN")]
    #[error("Invalid alignment: {alignment}")]
    InvalidAlignment { alignment: usize },

    #[classify(category = "internal", code = "MEM:ALLOC:MAX")]
    #[error("Allocation exceeds maximum size: {size} bytes (max: {max_size})")]
    ExceedsMaxSize { size: usize, max_size: usize },

    // --- Pool Errors ---
    #[classify(category = "exhausted", code = "MEM:POOL:EXHAUSTED")]
    #[error("Memory pool '{pool_id}' exhausted (capacity: {capacity})")]
    PoolExhausted { pool_id: Box<str>, capacity: usize },

    #[classify(category = "validation", code = "MEM:CONFIG:INVALID")]
    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: Box<str> },

    // --- Arena Errors ---
    #[classify(category = "exhausted", code = "MEM:ARENA:EXHAUSTED")]
    #[error("Arena '{arena_id}' exhausted: requested {requested} bytes, available {available}")]
    ArenaExhausted {
        arena_id: Box<str>,
        requested: usize,
        available: usize,
    },

    // --- Cache Errors ---
    #[classify(category = "not_found", code = "MEM:CACHE:MISS", retryable = true)]
    #[error("Cache miss for key: {key}")]
    CacheMiss { key: Box<str> },

    #[classify(category = "exhausted", code = "MEM:CACHE:OVERFLOW")]
    #[error("Cache overflow: {current} bytes used, {max} bytes maximum")]
    CacheOverflow { current: usize, max: usize },

    #[classify(category = "validation", code = "MEM:CACHE:KEY")]
    #[error("Invalid cache key: {reason}")]
    InvalidCacheKey { reason: Box<str> },

    // --- Budget Errors ---
    #[classify(category = "exhausted", code = "MEM:BUDGET:EXCEEDED")]
    #[error("Memory budget exceeded: {used} bytes used, {limit} bytes limit")]
    BudgetExceeded { used: usize, limit: usize },

    // --- System Errors ---
    #[classify(category = "internal", code = "MEM:SYSTEM:CORRUPTION")]
    #[error("Memory corruption detected in {component}: {details}")]
    Corruption {
        component: Box<str>,
        details: Box<str>,
    },

    #[classify(category = "internal", code = "MEM:SYSTEM:CONCURRENT")]
    #[error("Concurrent access error: {details}")]
    ConcurrentAccess { details: Box<str> },

    #[classify(category = "internal", code = "MEM:SYSTEM:STATE")]
    #[error("Invalid state: {reason}")]
    InvalidState { reason: Box<str> },

    #[classify(category = "internal", code = "MEM:SYSTEM:INIT")]
    #[error("Initialization failed: {reason}")]
    InitializationFailed { reason: Box<str> },

    // --- Feature Support Errors ---
    #[classify(category = "unsupported", code = "MEM:FEATURE:UNSUPPORTED")]
    #[error("Feature not supported: {feature}{}", context.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    NotSupported {
        feature: &'static str,
        context: Option<Box<str>>,
    },

    // --- General Errors ---
    #[classify(category = "not_found", code = "MEM:NOT_FOUND")]
    #[error("Operation not found: {reason}")]
    NotFound { reason: Box<str> },

    #[classify(category = "validation", code = "MEM:INVALID_OP")]
    #[error("Invalid operation: {reason}")]
    InvalidOperation { reason: Box<str> },
}

impl MemoryError {
    // ============================================================================
    // Convenience Constructors - Allocation Errors
    // ============================================================================

    /// Create allocation failed error
    #[must_use]
    pub fn allocation_failed(size: usize, align: usize) -> Self {
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
            reason: reason.into(),
        }
    }

    /// Create size overflow error
    pub fn size_overflow(operation: &str) -> Self {
        Self::SizeOverflow {
            operation: operation.into(),
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
        Self::PoolExhausted {
            pool_id: pool_id.into(),
            capacity,
        }
    }

    /// Create invalid pool config error
    pub fn invalid_pool_config(reason: &str) -> Self {
        Self::InvalidConfig {
            reason: format!("invalid pool config: {reason}").into_boxed_str(),
        }
    }

    /// Create invalid config error
    pub fn invalid_config(reason: &str) -> Self {
        Self::InvalidConfig {
            reason: reason.into(),
        }
    }

    // --- Arena Errors ---

    /// Create arena exhausted error
    pub fn arena_exhausted(arena_id: &str, requested: usize, available: usize) -> Self {
        Self::ArenaExhausted {
            arena_id: arena_id.into(),
            requested,
            available,
        }
    }

    /// Create invalid arena operation error
    pub fn invalid_arena_operation(operation: &str) -> Self {
        Self::InvalidState {
            reason: format!("invalid arena operation: {operation}").into_boxed_str(),
        }
    }

    // --- Cache Errors ---

    /// Create cache miss error
    pub fn cache_miss(key: &str) -> Self {
        Self::CacheMiss { key: key.into() }
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
            reason: format!("invalid key: {key}").into_boxed_str(),
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
            reason: format!("invalid budget: {reason}").into_boxed_str(),
        }
    }

    // --- System Errors ---

    /// Create memory corruption error
    pub fn corruption(component: &str, details: &str) -> Self {
        Self::Corruption {
            component: component.into(),
            details: details.into(),
        }
    }

    /// Create concurrent access error
    pub fn concurrent_access(details: &str) -> Self {
        Self::ConcurrentAccess {
            details: details.into(),
        }
    }

    /// Create leak detected error
    pub fn leak_detected(size: usize, location: &str) -> Self {
        Self::Corruption {
            component: "memory tracker".into(),
            details: format!("leak detected: {size} bytes at {location}").into_boxed_str(),
        }
    }

    /// Create fragmentation error
    #[must_use]
    pub fn fragmentation(available: usize, largest_block: usize, requested: usize) -> Self {
        Self::InvalidState {
            reason: format!(
                "fragmentation: {available} bytes available, largest block {largest_block}, requested {requested}"
            )
            .into_boxed_str(),
        }
    }

    /// Create initialization failed error
    pub fn initialization_failed(component: &str) -> Self {
        Self::InitializationFailed {
            reason: format!("failed to initialize {component}").into_boxed_str(),
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
            reason: format!("invalid index {index} (max: {max})").into_boxed_str(),
        }
    }

    /// Create decompression failed error
    pub fn decompression_failed(reason: &str) -> Self {
        Self::InvalidState {
            reason: format!("decompression failed: {reason}").into_boxed_str(),
        }
    }

    /// Create monitor error
    pub fn monitor_error(reason: &str) -> Self {
        Self::InvalidState {
            reason: format!("monitor error: {reason}").into_boxed_str(),
        }
    }

    /// Create not supported error
    #[must_use]
    pub fn not_supported(feature: &'static str) -> Self {
        Self::NotSupported {
            feature,
            context: None,
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_error::Classify;

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
