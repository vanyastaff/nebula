//! # Error Types for nebula-memory
//!
//! This module provides error handling for memory management operations,
//! using the nebula-error MemoryError kind directly.

use core::alloc::Layout;
use core::fmt;

pub use nebula_error::{
    NebulaError, Result as NebulaResult, kinds::MemoryError as MemoryErrorKind,
};

#[cfg(feature = "logging")]
use nebula_log::{debug, error, warn};

// ============================================================================
// Main Error Type
// ============================================================================

/// Memory management error type that wraps nebula-error MemoryError
#[derive(Debug, Clone)]
pub struct MemoryError {
    /// The underlying nebula error
    inner: NebulaError,
    /// Optional layout information for allocation errors
    layout: Option<Layout>,
}

impl MemoryError {
    /// Creates a new memory error from MemoryErrorKind
    pub fn new(kind: MemoryErrorKind) -> Self {
        #[cfg(feature = "logging")]
        {
            if kind.is_critical() {
                error!("Critical memory error: {}", kind);
            } else {
                warn!("Memory error: {}", kind);
            }
        }

        Self {
            inner: NebulaError::new(nebula_error::ErrorKind::Memory(kind)),
            layout: None,
        }
    }

    /// Creates an error with layout information
    pub fn with_layout(kind: MemoryErrorKind, layout: Layout) -> Self {
        let mut error = Self::new(kind);
        error.layout = Some(layout);
        error.inner = error.inner.with_details(format!(
            "layout: {} bytes, {} align",
            layout.size(),
            layout.align()
        ));
        error
    }

    /// Adds context to the error
    #[must_use = "builder methods must be chained or built"]
    pub fn with_context<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: fmt::Display,
    {
        self.inner = self
            .inner
            .with_details(format!("{}: {}", key.into(), value));
        self
    }

    /// Returns the error code string
    pub fn code(&self) -> &str {
        &self.inner.code
    }

    /// Returns the layout if available
    pub fn layout(&self) -> Option<Layout> {
        self.layout
    }

    /// Returns the underlying NebulaError
    pub fn inner(&self) -> &NebulaError {
        &self.inner
    }

    /// Converts into the underlying NebulaError
    pub fn into_inner(self) -> NebulaError {
        self.inner
    }

    // ============================================================================
    // Convenience Constructors - Allocation Errors
    // ============================================================================

    /// Creates an allocation failed error
    pub fn allocation_failed(size: usize, align: usize) -> Self {
        Self::new(MemoryErrorKind::AllocationFailed { size, align })
    }

    /// Creates an allocation failed error with layout
    pub fn allocation_failed_with_layout(layout: Layout) -> Self {
        Self::with_layout(
            MemoryErrorKind::AllocationFailed {
                size: layout.size(),
                align: layout.align(),
            },
            layout,
        )
    }

    /// Creates an invalid layout error
    pub fn invalid_layout(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidLayout {
            reason: reason.into(),
        })
    }

    /// Creates a size overflow error
    pub fn size_overflow(size: usize, align: usize) -> Self {
        Self::new(MemoryErrorKind::SizeOverflow {
            operation: format!("{}x{} byte allocation", size, align),
        })
    }

    /// Creates an invalid alignment error
    pub fn invalid_alignment(alignment: usize) -> Self {
        Self::new(MemoryErrorKind::InvalidAlignment { alignment })
    }

    // ============================================================================
    // Convenience Constructors - Pool Errors
    // ============================================================================

    /// Creates a pool exhausted error
    pub fn pool_exhausted(pool_id: impl Into<String>, capacity: usize) -> Self {
        Self::new(MemoryErrorKind::PoolExhausted {
            pool_id: pool_id.into(),
            capacity,
        })
    }

    /// Creates an invalid pool configuration error
    pub fn invalid_pool_config(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidConfig {
            reason: format!("invalid pool config: {}", reason.into()),
        })
    }

    /// Creates an invalid config error (generic)
    pub fn invalid_config(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidConfig {
            reason: reason.into(),
        })
    }

    // ============================================================================
    // Convenience Constructors - Arena Errors
    // ============================================================================

    /// Creates an arena exhausted error
    pub fn arena_exhausted(
        arena_id: impl Into<String>,
        requested: usize,
        available: usize,
    ) -> Self {
        Self::new(MemoryErrorKind::ArenaExhausted {
            arena_id: arena_id.into(),
            requested,
            available,
        })
    }

    /// Creates an invalid arena operation error
    pub fn invalid_arena_operation(operation: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!("invalid arena operation: {}", operation.into()),
        })
    }

    // ============================================================================
    // Convenience Constructors - Cache Errors
    // ============================================================================

    /// Creates a cache miss error
    pub fn cache_miss(key: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::CacheMiss { key: key.into() })
    }

    /// Creates a cache full error
    pub fn cache_full(capacity: usize) -> Self {
        Self::new(MemoryErrorKind::CacheOverflow {
            current: capacity,
            max: capacity,
        })
    }

    /// Creates an invalid cache key error
    pub fn invalid_cache_key(key: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidCacheKey {
            reason: format!("invalid key: {}", key.into()),
        })
    }

    // ============================================================================
    // Convenience Constructors - Budget Errors
    // ============================================================================

    /// Creates a budget exceeded error
    pub fn budget_exceeded(used: usize, limit: usize) -> Self {
        Self::new(MemoryErrorKind::BudgetExceeded { used, limit })
    }

    /// Creates an invalid budget error
    pub fn invalid_budget(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidConfig {
            reason: format!("invalid budget: {}", reason.into()),
        })
    }

    // ============================================================================
    // Convenience Constructors - System Errors
    // ============================================================================

    /// Creates a memory corruption error
    pub fn corruption(component: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::Corruption {
            component: component.into(),
            details: details.into(),
        })
    }

    /// Creates a concurrent access error
    pub fn concurrent_access(resource: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::ConcurrentAccess {
            details: format!("concurrent access to {}", resource.into()),
        })
    }

    /// Creates a leak detected error
    pub fn leak_detected(size: usize, location: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::Corruption {
            component: "memory tracker".into(),
            details: format!("leak detected: {} bytes at {}", size, location.into()),
        })
    }

    /// Creates a fragmentation error
    pub fn fragmentation(available: usize, largest_block: usize, requested: usize) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!(
                "fragmentation: {} bytes available, largest block {}, requested {}",
                available, largest_block, requested
            ),
        })
    }

    /// Creates an initialization failed error
    pub fn initialization_failed(component: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InitializationFailed {
            reason: format!("failed to initialize {}", component.into()),
        })
    }

    /// Creates an invalid input error
    pub fn invalid_input(reason: impl Into<String>) -> Self {
        Self::invalid_layout(reason)
    }

    /// Creates an invalid argument error (alias for invalid_input)
    pub fn invalid_argument(reason: impl Into<String>) -> Self {
        Self::invalid_input(reason)
    }

    /// Creates an invalid index error
    pub fn invalid_index(index: usize, max: usize) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!("invalid index {} (max: {})", index, max),
        })
    }

    /// Creates a decompression failed error
    pub fn decompression_failed(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!("decompression failed: {}", reason.into()),
        })
    }

    /// Creates a monitor error
    pub fn monitor_error(reason: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!("monitor error: {}", reason.into()),
        })
    }

    /// Creates a not supported error
    pub fn not_supported(operation: impl Into<String>) -> Self {
        Self::new(MemoryErrorKind::InvalidState {
            reason: format!("operation not supported: {}", operation.into()),
        })
    }

    /// Creates an out of memory error
    pub fn out_of_memory(size: usize, align: usize) -> Self {
        Self::allocation_failed(size, align)
    }

    /// Creates an out of memory error with layout
    pub fn out_of_memory_with_layout(layout: Layout) -> Self {
        Self::allocation_failed_with_layout(layout)
    }

    /// Creates an allocation too large error
    pub fn allocation_too_large(size: usize, max_size: usize) -> Self {
        Self::new(MemoryErrorKind::ExceedsMaxSize { size, max_size })
    }
}

impl From<MemoryErrorKind> for MemoryError {
    fn from(kind: MemoryErrorKind) -> Self {
        Self::new(kind)
    }
}

impl From<MemoryError> for NebulaError {
    fn from(error: MemoryError) -> Self {
        error.inner
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(layout) = self.layout {
            write!(
                f,
                "Memory error for {} bytes with {} alignment: {}",
                layout.size(),
                layout.align(),
                self.inner
            )
        } else {
            write!(f, "{}", self.inner)
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

// ============================================================================
// Result Type and Utilities
// ============================================================================

/// Result type for memory operations
pub type MemoryResult<T> = Result<T, MemoryError>;

// Type aliases for backward compatibility with allocator module
pub type AllocError = MemoryError;
pub type AllocResult<T> = MemoryResult<T>;

/// Extension trait for Result types
pub trait MemoryResultExt<T> {
    /// Maps a memory error with additional context
    fn map_memory_err<F>(self, f: F) -> MemoryResult<T>
    where
        F: FnOnce(MemoryError) -> MemoryError;

    /// Adds context message to memory error
    fn context(self, msg: &str) -> MemoryResult<T>;
}

impl<T> MemoryResultExt<T> for MemoryResult<T> {
    fn map_memory_err<F>(self, f: F) -> MemoryResult<T>
    where
        F: FnOnce(MemoryError) -> MemoryError,
    {
        self.map_err(f)
    }

    fn context(self, msg: &str) -> MemoryResult<T> {
        self.map_err(|e| e.with_context("context", msg))
    }
}

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
    }

    #[test]
    fn test_error_with_layout() {
        let layout = Layout::new::<u64>();
        let error = MemoryError::allocation_failed_with_layout(layout);

        assert_eq!(error.layout(), Some(layout));
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
    }

    #[test]
    fn test_budget_errors() {
        let error = MemoryError::budget_exceeded(2048, 1024);
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn test_corruption_errors() {
        let error = MemoryError::corruption("allocator", "invalid pointer");
        assert!(!error.to_string().is_empty());
    }
}
