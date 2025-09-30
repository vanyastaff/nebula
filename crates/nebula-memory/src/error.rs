//! # Error Types for nebula-memory
//!
//! This module provides comprehensive error handling for all memory management
//! operations in the nebula-memory crate, following nebula-error patterns.

use core::alloc::Layout;
use core::fmt;

use nebula_error::{
    core::traits::ErrorCode,
    NebulaError,
    ErrorContext,
    ErrorKind,
    kinds::SystemError,
    Result as NebulaResult
};

#[cfg(feature = "logging")]
use nebula_log::{error, warn, debug};

// ============================================================================
// Error Codes
// ============================================================================

/// Memory management error codes following nebula-error patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryErrorCode {
    // Allocation errors
    /// Memory allocation failed due to insufficient memory
    AllocationFailed,
    /// Invalid memory layout parameters
    InvalidLayout,
    /// Size calculation overflow
    SizeOverflow,
    /// Invalid alignment (not a power of two)
    InvalidAlignment,
    /// Allocation exceeds maximum supported size
    ExceedsMaxSize,

    // Pool errors
    /// Object pool is exhausted
    PoolExhausted,
    /// Invalid pool configuration
    InvalidPoolConfig,
    /// Pool corruption detected
    PoolCorruption,

    // Arena errors
    /// Arena has insufficient space
    ArenaExhausted,
    /// Invalid arena operation
    InvalidArenaOperation,
    /// Arena memory corruption
    ArenaCorruption,

    // Cache errors
    /// Cache miss
    CacheMiss,
    /// Cache capacity exceeded
    CacheOverflow,
    /// Invalid cache key
    InvalidCacheKey,
    /// Cache corruption
    CacheCorruption,

    // Budget errors
    /// Memory budget exceeded
    BudgetExceeded,
    /// Invalid budget configuration
    InvalidBudget,

    // System errors
    /// Memory system initialization failed
    InitializationFailed,
    /// Invalid system state
    InvalidState,
    /// Concurrent access violation
    ConcurrentAccess,
    /// Resource limit exceeded
    ResourceLimit,

    // Configuration errors
    /// Invalid configuration parameters
    InvalidConfig,
    /// Configuration not found
    ConfigNotFound,
}

impl ErrorCode for MemoryErrorCode {
    fn error_code(&self) -> &str {
        match self {
            // Allocation errors
            MemoryErrorCode::AllocationFailed => "MEMORY_ALLOCATION_FAILED",
            MemoryErrorCode::InvalidLayout => "MEMORY_INVALID_LAYOUT",
            MemoryErrorCode::SizeOverflow => "MEMORY_SIZE_OVERFLOW",
            MemoryErrorCode::InvalidAlignment => "MEMORY_INVALID_ALIGNMENT",
            MemoryErrorCode::ExceedsMaxSize => "MEMORY_EXCEEDS_MAX_SIZE",

            // Pool errors
            MemoryErrorCode::PoolExhausted => "MEMORY_POOL_EXHAUSTED",
            MemoryErrorCode::InvalidPoolConfig => "MEMORY_INVALID_POOL_CONFIG",
            MemoryErrorCode::PoolCorruption => "MEMORY_POOL_CORRUPTION",

            // Arena errors
            MemoryErrorCode::ArenaExhausted => "MEMORY_ARENA_EXHAUSTED",
            MemoryErrorCode::InvalidArenaOperation => "MEMORY_INVALID_ARENA_OPERATION",
            MemoryErrorCode::ArenaCorruption => "MEMORY_ARENA_CORRUPTION",

            // Cache errors
            MemoryErrorCode::CacheMiss => "MEMORY_CACHE_MISS",
            MemoryErrorCode::CacheOverflow => "MEMORY_CACHE_OVERFLOW",
            MemoryErrorCode::InvalidCacheKey => "MEMORY_INVALID_CACHE_KEY",
            MemoryErrorCode::CacheCorruption => "MEMORY_CACHE_CORRUPTION",

            // Budget errors
            MemoryErrorCode::BudgetExceeded => "MEMORY_BUDGET_EXCEEDED",
            MemoryErrorCode::InvalidBudget => "MEMORY_INVALID_BUDGET",

            // System errors
            MemoryErrorCode::InitializationFailed => "MEMORY_INITIALIZATION_FAILED",
            MemoryErrorCode::InvalidState => "MEMORY_INVALID_STATE",
            MemoryErrorCode::ConcurrentAccess => "MEMORY_CONCURRENT_ACCESS",
            MemoryErrorCode::ResourceLimit => "MEMORY_RESOURCE_LIMIT",

            // Configuration errors
            MemoryErrorCode::InvalidConfig => "MEMORY_INVALID_CONFIG",
            MemoryErrorCode::ConfigNotFound => "MEMORY_CONFIG_NOT_FOUND",
        }
    }

    fn error_category(&self) -> &str {
        match self {
            MemoryErrorCode::AllocationFailed
            | MemoryErrorCode::InvalidLayout
            | MemoryErrorCode::SizeOverflow
            | MemoryErrorCode::InvalidAlignment
            | MemoryErrorCode::ExceedsMaxSize => "memory.allocation",

            MemoryErrorCode::PoolExhausted
            | MemoryErrorCode::InvalidPoolConfig
            | MemoryErrorCode::PoolCorruption => "memory.pool",

            MemoryErrorCode::ArenaExhausted
            | MemoryErrorCode::InvalidArenaOperation
            | MemoryErrorCode::ArenaCorruption => "memory.arena",

            MemoryErrorCode::CacheMiss
            | MemoryErrorCode::CacheOverflow
            | MemoryErrorCode::InvalidCacheKey
            | MemoryErrorCode::CacheCorruption => "memory.cache",

            MemoryErrorCode::BudgetExceeded
            | MemoryErrorCode::InvalidBudget => "memory.budget",

            MemoryErrorCode::InitializationFailed
            | MemoryErrorCode::InvalidState
            | MemoryErrorCode::ConcurrentAccess
            | MemoryErrorCode::ResourceLimit => "memory.system",

            MemoryErrorCode::InvalidConfig
            | MemoryErrorCode::ConfigNotFound => "memory.config",
        }
    }
}

impl MemoryErrorCode {
    /// Convert to nebula error kind
    pub fn to_error_kind(self) -> ErrorKind {
        match self {
            MemoryErrorCode::AllocationFailed => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory".to_string(),
            }),
            MemoryErrorCode::InvalidLayout => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory layout".to_string(),
            }),
            MemoryErrorCode::SizeOverflow => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory size calculation overflow".to_string(),
            }),
            MemoryErrorCode::InvalidAlignment => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory alignment".to_string(),
            }),
            MemoryErrorCode::ExceedsMaxSize => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory size exceeds maximum".to_string(),
            }),
            MemoryErrorCode::PoolExhausted => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory pool exhausted".to_string(),
            }),
            MemoryErrorCode::InvalidPoolConfig => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid pool configuration".to_string(),
            }),
            MemoryErrorCode::PoolCorruption => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory pool corruption".to_string(),
            }),
            MemoryErrorCode::ArenaExhausted => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory arena exhausted".to_string(),
            }),
            MemoryErrorCode::InvalidArenaOperation => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid arena operation".to_string(),
            }),
            MemoryErrorCode::ArenaCorruption => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory arena corruption".to_string(),
            }),
            MemoryErrorCode::CacheMiss => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "cache miss".to_string(),
            }),
            MemoryErrorCode::CacheOverflow => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "cache overflow".to_string(),
            }),
            MemoryErrorCode::InvalidCacheKey => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid cache key".to_string(),
            }),
            MemoryErrorCode::CacheCorruption => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "cache corruption".to_string(),
            }),
            MemoryErrorCode::BudgetExceeded => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory budget exceeded".to_string(),
            }),
            MemoryErrorCode::InvalidBudget => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid budget configuration".to_string(),
            }),
            MemoryErrorCode::InitializationFailed => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory system initialization failed".to_string(),
            }),
            MemoryErrorCode::InvalidState => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory system state".to_string(),
            }),
            MemoryErrorCode::ConcurrentAccess => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "concurrent access violation".to_string(),
            }),
            MemoryErrorCode::ResourceLimit => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory resource limit exceeded".to_string(),
            }),
            MemoryErrorCode::InvalidConfig => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory configuration".to_string(),
            }),
            MemoryErrorCode::ConfigNotFound => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory configuration not found".to_string(),
            }),
        }
    }
    /// Get human-readable error message
    pub fn message(&self) -> &'static str {
        match self {
            // Allocation errors
            MemoryErrorCode::AllocationFailed => "Memory allocation failed due to insufficient memory",
            MemoryErrorCode::InvalidLayout => "Memory layout parameters are invalid",
            MemoryErrorCode::SizeOverflow => "Memory size calculation overflowed",
            MemoryErrorCode::InvalidAlignment => "Memory alignment must be a power of two",
            MemoryErrorCode::ExceedsMaxSize => "Memory allocation size exceeds maximum supported size",

            // Pool errors
            MemoryErrorCode::PoolExhausted => "Object pool has no available objects",
            MemoryErrorCode::InvalidPoolConfig => "Object pool configuration is invalid",
            MemoryErrorCode::PoolCorruption => "Object pool internal corruption detected",

            // Arena errors
            MemoryErrorCode::ArenaExhausted => "Memory arena has insufficient space",
            MemoryErrorCode::InvalidArenaOperation => "Invalid operation on memory arena",
            MemoryErrorCode::ArenaCorruption => "Memory arena corruption detected",

            // Cache errors
            MemoryErrorCode::CacheMiss => "Requested key not found in cache",
            MemoryErrorCode::CacheOverflow => "Cache capacity exceeded",
            MemoryErrorCode::InvalidCacheKey => "Cache key is invalid",
            MemoryErrorCode::CacheCorruption => "Cache internal corruption detected",

            // Budget errors
            MemoryErrorCode::BudgetExceeded => "Memory budget limit exceeded",
            MemoryErrorCode::InvalidBudget => "Memory budget configuration is invalid",

            // System errors
            MemoryErrorCode::InitializationFailed => "Memory system initialization failed",
            MemoryErrorCode::InvalidState => "Memory system is in an invalid state",
            MemoryErrorCode::ConcurrentAccess => "Unsafe concurrent access to memory system",
            MemoryErrorCode::ResourceLimit => "System resource limit exceeded",

            // Configuration errors
            MemoryErrorCode::InvalidConfig => "Memory configuration parameters are invalid",
            MemoryErrorCode::ConfigNotFound => "Memory configuration not found",
        }
    }

    /// Get error severity level
    pub fn severity(&self) -> Severity {
        match self {
            // Critical errors that can crash the system
            MemoryErrorCode::AllocationFailed
            | MemoryErrorCode::PoolCorruption
            | MemoryErrorCode::ArenaCorruption
            | MemoryErrorCode::CacheCorruption
            | MemoryErrorCode::ConcurrentAccess => Severity::Critical,

            // Errors that prevent operation but are recoverable
            MemoryErrorCode::InvalidLayout
            | MemoryErrorCode::SizeOverflow
            | MemoryErrorCode::InvalidAlignment
            | MemoryErrorCode::InvalidPoolConfig
            | MemoryErrorCode::InvalidArenaOperation
            | MemoryErrorCode::InvalidCacheKey
            | MemoryErrorCode::InvalidBudget
            | MemoryErrorCode::InitializationFailed
            | MemoryErrorCode::InvalidState
            | MemoryErrorCode::InvalidConfig
            | MemoryErrorCode::ConfigNotFound => Severity::Error,

            // Warnings that don't prevent operation
            MemoryErrorCode::ExceedsMaxSize
            | MemoryErrorCode::PoolExhausted
            | MemoryErrorCode::ArenaExhausted
            | MemoryErrorCode::CacheOverflow
            | MemoryErrorCode::BudgetExceeded
            | MemoryErrorCode::ResourceLimit => Severity::Warning,

            // Informational (cache miss is normal)
            MemoryErrorCode::CacheMiss => Severity::Info,
        }
    }
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational message
    Info,
    /// Warning that doesn't prevent operation
    Warning,
    /// Error that prevents operation but is recoverable
    Error,
    /// Critical error that may crash the system
    Critical,
}

// ============================================================================
// Main Error Type
// ============================================================================

/// Memory management error type that integrates with nebula-error system
#[derive(Debug, Clone)]
pub struct MemoryError {
    /// The underlying nebula error
    inner: NebulaError,
    /// Optional layout information for allocation errors
    layout: Option<Layout>,
    /// Optional size information
    size: Option<usize>,
}

impl MemoryError {
    /// Creates a new memory error with specific code
    pub fn new(code: MemoryErrorCode) -> Self {
        #[cfg(feature = "logging")]
        {
            match code.severity() {
                Severity::Critical => {
                    error!("Critical memory error: {}", code.message());
                }
                Severity::Error => {
                    error!("Memory error: {}", code.message());
                }
                Severity::Warning => {
                    warn!("Memory warning: {}", code.message());
                }
                Severity::Info => {
                    debug!("Memory info: {}", code.message());
                }
            }
        }

        Self {
            inner: NebulaError::new(code.to_error_kind()),
            layout: None,
            size: None,
        }
    }

    /// Creates an error with layout information
    pub fn with_layout(code: MemoryErrorCode, layout: Layout) -> Self {
        let mut error = Self::new(code);
        error.layout = Some(layout);
        error.inner = error.inner
;
        error
    }

    /// Creates an error with size information
    pub fn with_size(code: MemoryErrorCode, size: usize) -> Self {
        let mut error = Self::new(code);
        error.size = Some(size);
        error
    }

    /// Adds context to the error
    #[cfg(feature = "std")]
    pub fn with_context<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: fmt::Display,
    {
        use std::collections::HashMap;
        let mut metadata = HashMap::new();
        metadata.insert(key.into(), value.to_string());

        let context = ErrorContext {
            description: "Memory error context".to_string(),
            metadata,
            stack_trace: None,
            timestamp: Some(chrono::Utc::now()),
            user_id: None,
            tenant_id: None,
            request_id: None,
            component: Some("nebula-memory".to_string()),
            operation: Some("memory".to_string()),
        };

        self.inner = self.inner.with_context(context);
        self
    }

    /// Adds context to the error (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn with_context<K, V>(self, _key: K, _value: V) -> Self
    where
        K: Into<String>,
        V: fmt::Display,
    {
        // No-std version can't create full context
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

    /// Returns the size if available
    pub fn size(&self) -> Option<usize> {
        self.size
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
    // Convenience Constructors
    // ============================================================================

    /// Creates an allocation failed error
    pub fn allocation_failed() -> Self {
        Self::new(MemoryErrorCode::AllocationFailed)
    }

    /// Creates an allocation failed error with layout
    pub fn allocation_failed_with_layout(layout: Layout) -> Self {
        Self::with_layout(MemoryErrorCode::AllocationFailed, layout)
    }

    /// Creates an invalid layout error
    pub fn invalid_layout() -> Self {
        Self::new(MemoryErrorCode::InvalidLayout)
    }

    /// Creates a size overflow error
    pub fn size_overflow() -> Self {
        Self::new(MemoryErrorCode::SizeOverflow)
    }

    /// Creates a pool exhausted error
    pub fn pool_exhausted() -> Self {
        Self::new(MemoryErrorCode::PoolExhausted)
    }

    /// Creates an arena exhausted error
    pub fn arena_exhausted() -> Self {
        Self::new(MemoryErrorCode::ArenaExhausted)
    }

    /// Creates a cache miss error
    pub fn cache_miss() -> Self {
        Self::new(MemoryErrorCode::CacheMiss)
    }

    /// Creates a budget exceeded error
    pub fn budget_exceeded() -> Self {
        Self::new(MemoryErrorCode::BudgetExceeded)
    }

    /// Creates an initialization failed error
    pub fn initialization_failed<T: fmt::Display>(details: T) -> Self {
        Self::new(MemoryErrorCode::InitializationFailed)
            .with_context("details", details)
    }

    /// Creates an invalid config error
    pub fn invalid_config<T: fmt::Display>(details: T) -> Self {
        Self::new(MemoryErrorCode::InvalidConfig)
            .with_context("details", details)
    }
}

impl From<MemoryErrorCode> for MemoryError {
    fn from(code: MemoryErrorCode) -> Self {
        Self::new(code)
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
            write!(f, "Memory error for {} bytes with {} alignment: {}",
                   layout.size(), layout.align(), self.inner)
        } else if let Some(size) = self.size {
            write!(f, "Memory error for {} bytes: {}", size, self.inner)
        } else {
            write!(f, "Memory error: {}", self.inner)
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
        let error = MemoryError::new(MemoryErrorCode::AllocationFailed);
        let code = MemoryErrorCode::AllocationFailed;
        assert_eq!(code.error_code(), "MEMORY_ALLOCATION_FAILED");
        assert_eq!(code.severity(), Severity::Critical);
    }

    #[test]
    fn test_error_with_layout() {
        let layout = Layout::new::<u64>();
        let error = MemoryError::with_layout(MemoryErrorCode::InvalidLayout, layout);

        assert_eq!(error.layout(), Some(layout));
        let code = MemoryErrorCode::InvalidLayout;
        assert_eq!(code.error_code(), "MEMORY_INVALID_LAYOUT");
    }

    #[test]
    fn test_convenience_constructors() {
        let alloc_error = MemoryError::allocation_failed();
        let pool_error = MemoryError::pool_exhausted();
        let cache_error = MemoryError::cache_miss();

        assert!(!alloc_error.to_string().is_empty());
        assert!(!pool_error.to_string().is_empty());
        assert!(!cache_error.to_string().is_empty());
    }

    #[test]
    fn test_error_categories() {
        assert_eq!(MemoryErrorCode::AllocationFailed.error_category(), "memory.allocation");
        assert_eq!(MemoryErrorCode::PoolExhausted.error_category(), "memory.pool");
        assert_eq!(MemoryErrorCode::ArenaExhausted.error_category(), "memory.arena");
        assert_eq!(MemoryErrorCode::CacheMiss.error_category(), "memory.cache");
    }

    #[test]
    fn test_severity_levels() {
        assert_eq!(MemoryErrorCode::AllocationFailed.severity(), Severity::Critical);
        assert_eq!(MemoryErrorCode::InvalidLayout.severity(), Severity::Error);
        assert_eq!(MemoryErrorCode::PoolExhausted.severity(), Severity::Warning);
        assert_eq!(MemoryErrorCode::CacheMiss.severity(), Severity::Info);
    }
}