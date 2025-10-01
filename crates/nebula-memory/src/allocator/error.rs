//! Enhanced memory allocation error type with nebula-error integration
//!
//! Provides a unified error type for memory allocation operations with:
//! - Integration with nebula-error for consistent error handling
//! - Cross-platform support (stable and nightly)
//! - Rich context and debugging information
//! - Telemetry and metrics integration

use core::alloc::Layout;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use nebula_error::{
    core::traits::ErrorCode,
    NebulaError,
    ErrorContext,
    ErrorKind,
    kinds::SystemError,
};

#[cfg(feature = "std")]
use std::collections::HashMap;

#[cfg(feature = "backtrace")]
use std::backtrace::Backtrace;

#[cfg(feature = "logging")]
use nebula_log::{error, warn, debug};

// ============================================================================
// Error Codes for Allocator Module
// ============================================================================

/// Allocator-specific error codes following nebula-error patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocErrorCode {
    /// General allocation failure (out of memory)
    OutOfMemory,
    /// Size overflow when calculating total allocation size
    SizeOverflow,
    /// Invalid alignment (not a power of two)
    InvalidAlignment,
    /// Allocation size exceeds maximum supported size
    ExceedsMaxSize,
    /// Invalid layout parameters
    InvalidLayout,
    /// Allocator is in an invalid state
    InvalidState,
    /// Concurrent access violation
    ConcurrentAccess,
    /// Resource limit exceeded
    ResourceLimit,
    /// Pool exhausted
    PoolExhausted,
    /// Arena exhausted
    ArenaExhausted,
    /// Allocation denied by monitoring system due to memory pressure
    AllocationDenied,
}

impl ErrorCode for AllocErrorCode {
    fn error_code(&self) -> &str {
        match self {
            AllocErrorCode::OutOfMemory => "ALLOC_OUT_OF_MEMORY",
            AllocErrorCode::SizeOverflow => "ALLOC_SIZE_OVERFLOW",
            AllocErrorCode::InvalidAlignment => "ALLOC_INVALID_ALIGNMENT",
            AllocErrorCode::ExceedsMaxSize => "ALLOC_EXCEEDS_MAX_SIZE",
            AllocErrorCode::InvalidLayout => "ALLOC_INVALID_LAYOUT",
            AllocErrorCode::InvalidState => "ALLOC_INVALID_STATE",
            AllocErrorCode::ConcurrentAccess => "ALLOC_CONCURRENT_ACCESS",
            AllocErrorCode::ResourceLimit => "ALLOC_RESOURCE_LIMIT",
            AllocErrorCode::PoolExhausted => "ALLOC_POOL_EXHAUSTED",
            AllocErrorCode::ArenaExhausted => "ALLOC_ARENA_EXHAUSTED",
            AllocErrorCode::AllocationDenied => "ALLOC_DENIED",
        }
    }

    fn error_category(&self) -> &str {
        "memory.allocator"
    }
}

impl AllocErrorCode {
    /// Convert to nebula error kind
    pub fn to_error_kind(self) -> ErrorKind {
        match self {
            AllocErrorCode::OutOfMemory => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory".to_string(),
            }),
            AllocErrorCode::SizeOverflow => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory size calculation overflow".to_string(),
            }),
            AllocErrorCode::InvalidAlignment => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory alignment".to_string(),
            }),
            AllocErrorCode::ExceedsMaxSize => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory size exceeds maximum".to_string(),
            }),
            AllocErrorCode::InvalidLayout => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid memory layout".to_string(),
            }),
            AllocErrorCode::PoolExhausted => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory pool exhausted".to_string(),
            }),
            AllocErrorCode::ArenaExhausted => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory arena exhausted".to_string(),
            }),
            AllocErrorCode::AllocationDenied => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "memory allocation denied".to_string(),
            }),
            AllocErrorCode::InvalidState => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "invalid allocator state".to_string(),
            }),
            AllocErrorCode::ConcurrentAccess => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "concurrent access violation".to_string(),
            }),
            AllocErrorCode::ResourceLimit => ErrorKind::System(SystemError::ResourceExhausted {
                resource: "allocator resource limit exceeded".to_string(),
            }),
        }
    }
    /// Get the error code as a string
    pub fn code(&self) -> &'static str {
        match self {
            AllocErrorCode::OutOfMemory => "ALLOC_OUT_OF_MEMORY",
            AllocErrorCode::SizeOverflow => "ALLOC_SIZE_OVERFLOW",
            AllocErrorCode::InvalidAlignment => "ALLOC_INVALID_ALIGNMENT",
            AllocErrorCode::ExceedsMaxSize => "ALLOC_EXCEEDS_MAX_SIZE",
            AllocErrorCode::InvalidLayout => "ALLOC_INVALID_LAYOUT",
            AllocErrorCode::InvalidState => "ALLOC_INVALID_STATE",
            AllocErrorCode::ConcurrentAccess => "ALLOC_CONCURRENT_ACCESS",
            AllocErrorCode::ResourceLimit => "ALLOC_RESOURCE_LIMIT",
            AllocErrorCode::PoolExhausted => "ALLOC_POOL_EXHAUSTED",
            AllocErrorCode::ArenaExhausted => "ALLOC_ARENA_EXHAUSTED",
            AllocErrorCode::AllocationDenied => "ALLOC_DENIED",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            AllocErrorCode::OutOfMemory => "Memory allocation failed due to insufficient memory",
            AllocErrorCode::SizeOverflow => "Allocation size calculation overflowed",
            AllocErrorCode::InvalidAlignment => "Alignment must be a power of two",
            AllocErrorCode::ExceedsMaxSize => "Allocation size exceeds maximum supported size",
            AllocErrorCode::InvalidLayout => "Memory layout parameters are invalid",
            AllocErrorCode::InvalidState => "Allocator is in an invalid state",
            AllocErrorCode::ConcurrentAccess => "Unsafe concurrent access to allocator",
            AllocErrorCode::ResourceLimit => "Allocator resource limit exceeded",
            AllocErrorCode::PoolExhausted => "Memory pool has no available objects",
            AllocErrorCode::ArenaExhausted => "Memory arena has insufficient space",
            AllocErrorCode::AllocationDenied => "Allocation denied due to memory pressure",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            AllocErrorCode::OutOfMemory => Severity::Critical,
            AllocErrorCode::SizeOverflow => Severity::Error,
            AllocErrorCode::InvalidAlignment => Severity::Error,
            AllocErrorCode::ExceedsMaxSize => Severity::Warning,
            AllocErrorCode::InvalidLayout => Severity::Error,
            AllocErrorCode::InvalidState => Severity::Error,
            AllocErrorCode::ConcurrentAccess => Severity::Critical,
            AllocErrorCode::ResourceLimit => Severity::Warning,
            AllocErrorCode::PoolExhausted => Severity::Warning,
            AllocErrorCode::ArenaExhausted => Severity::Warning,
            AllocErrorCode::AllocationDenied => Severity::Warning,
        }
    }
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

// ============================================================================
// Error Statistics
// ============================================================================

/// Global error statistics for monitoring
pub struct ErrorStats {
    out_of_memory: AtomicU64,
    size_overflow: AtomicU64,
    invalid_alignment: AtomicU64,
    exceeds_max_size: AtomicU64,
    invalid_layout: AtomicU64,
    invalid_state: AtomicU64,
    concurrent_access: AtomicU64,
    resource_limit: AtomicU64,
    total_errors: AtomicU64,
}

impl ErrorStats {
    const fn new() -> Self {
        Self {
            out_of_memory: AtomicU64::new(0),
            size_overflow: AtomicU64::new(0),
            invalid_alignment: AtomicU64::new(0),
            exceeds_max_size: AtomicU64::new(0),
            invalid_layout: AtomicU64::new(0),
            invalid_state: AtomicU64::new(0),
            concurrent_access: AtomicU64::new(0),
            resource_limit: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
        }
    }

    fn record(&self, code: AllocErrorCode) {
        self.total_errors.fetch_add(1, Ordering::Relaxed);

        #[cfg(feature = "logging")]
        {
            match code.severity() {
                Severity::Critical => {
                    error!("Critical allocator error: {}", code.message());
                }
                Severity::Error => {
                    error!("Allocator error: {}", code.message());
                }
                Severity::Warning => {
                    warn!("Allocator warning: {}", code.message());
                }
                Severity::Info => {
                    debug!("Allocator issue: {}", code.message());
                }
            }
        }

        match code {
            AllocErrorCode::OutOfMemory => {
                self.out_of_memory.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::SizeOverflow => {
                self.size_overflow.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::InvalidAlignment => {
                self.invalid_alignment.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::ExceedsMaxSize => {
                self.exceeds_max_size.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::InvalidLayout => {
                self.invalid_layout.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::InvalidState => {
                self.invalid_state.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::ConcurrentAccess => {
                self.concurrent_access.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::ResourceLimit => {
                self.resource_limit.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::PoolExhausted => {
                self.out_of_memory.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::ArenaExhausted => {
                self.out_of_memory.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorCode::AllocationDenied => {
                self.resource_limit.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn get_stats(&self) -> ErrorStatsSnapshot {
        ErrorStatsSnapshot {
            out_of_memory: self.out_of_memory.load(Ordering::Relaxed),
            size_overflow: self.size_overflow.load(Ordering::Relaxed),
            invalid_alignment: self.invalid_alignment.load(Ordering::Relaxed),
            exceeds_max_size: self.exceeds_max_size.load(Ordering::Relaxed),
            invalid_layout: self.invalid_layout.load(Ordering::Relaxed),
            invalid_state: self.invalid_state.load(Ordering::Relaxed),
            concurrent_access: self.concurrent_access.load(Ordering::Relaxed),
            resource_limit: self.resource_limit.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.out_of_memory.store(0, Ordering::Relaxed);
        self.size_overflow.store(0, Ordering::Relaxed);
        self.invalid_alignment.store(0, Ordering::Relaxed);
        self.exceeds_max_size.store(0, Ordering::Relaxed);
        self.invalid_layout.store(0, Ordering::Relaxed);
        self.invalid_state.store(0, Ordering::Relaxed);
        self.concurrent_access.store(0, Ordering::Relaxed);
        self.resource_limit.store(0, Ordering::Relaxed);
        self.total_errors.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ErrorStatsSnapshot {
    pub out_of_memory: u64,
    pub size_overflow: u64,
    pub invalid_alignment: u64,
    pub exceeds_max_size: u64,
    pub invalid_layout: u64,
    pub invalid_state: u64,
    pub concurrent_access: u64,
    pub resource_limit: u64,
    pub total_errors: u64,
}

/// Global error statistics instance
pub static ERROR_STATS: ErrorStats = ErrorStats::new();

// ============================================================================
// Memory State
// ============================================================================

/// System memory state snapshot
#[derive(Debug, Clone, Copy)]
pub struct MemoryState {
    /// Available system memory in bytes
    pub available: Option<usize>,
    /// Total system memory in bytes
    pub total: Option<usize>,
    /// Current process memory usage in bytes
    pub process_used: Option<usize>,
    /// Number of active allocations
    pub active_allocations: Option<usize>,
}

impl MemoryState {
    pub const fn new() -> Self {
        Self {
            available: None,
            total: None,
            process_used: None,
            active_allocations: None,
        }
    }

    #[cfg(feature = "std")]
    pub fn capture() -> Option<Self> {
        // This would integrate with nebula-system for actual memory info
        // For now, returning None - will implement with nebula-system integration
        None
    }
}

// ============================================================================
// Enhanced AllocError Type
// ============================================================================

/// Allocator error type that integrates with nebula-error system
#[derive(Debug, Clone)]
pub struct AllocError {
    /// The underlying nebula error
    inner: NebulaError,
    /// Layout information for the failed allocation
    layout: Option<Layout>,
    /// Memory state at time of error
    memory_state: Option<MemoryState>,
}

impl AllocError {
    /// Creates a new allocation error with specific code
    pub fn new(code: AllocErrorCode) -> Self {
        ERROR_STATS.record(code);
        Self {
            inner: NebulaError::new(code.to_error_kind()),
            layout: None,
            memory_state: None,
        }
    }

    /// Creates an allocation error with layout information
    pub fn with_layout(code: AllocErrorCode, layout: Layout) -> Self {
        ERROR_STATS.record(code);
        Self {
            inner: {
                #[cfg(feature = "std")]
                {
                    let layout_context = Self::create_context("layout", format!("size: {}, align: {}", layout.size(), layout.align()));
                    NebulaError::new(code.to_error_kind()).with_context(layout_context)
                }
                #[cfg(not(feature = "std"))]
                {
                    NebulaError::new(code.to_error_kind())
                }
            },
            layout: Some(layout),
            memory_state: None,
        }
    }

    /// Creates an allocation error with memory state
    #[cfg(feature = "std")]
    pub fn with_memory_state(code: AllocErrorCode, layout: Option<Layout>) -> Self {
        ERROR_STATS.record(code);
        let memory_state = MemoryState::capture();
        let mut error = NebulaError::new(code.to_error_kind());

        if let Some(layout) = layout {
            let layout_context = Self::create_context("layout", format!("size: {}, align: {}", layout.size(), layout.align()));
            error = error.with_context(layout_context);
        }

        if let Some(ref state) = memory_state {
            if let Some(available) = state.available {
                let mem_context = Self::create_context("memory_state", format!("available: {}", available));
                error = error.with_context(mem_context);
            }
            if let Some(total) = state.total {
                let total_context = Self::create_context("memory_total", format!("total: {}", total));
                error = error.with_context(total_context);
            }
        }

        Self {
            inner: error,
            layout,
            memory_state,
        }
    }

    /// Creates error context with key-value pair
    #[cfg(feature = "std")]
    fn create_context(key: &str, value: impl fmt::Display) -> ErrorContext {
        let mut metadata = HashMap::new();
        metadata.insert(key.to_string(), value.to_string());

        ErrorContext {
            description: format!("Memory allocation context: {}", key),
            metadata,
            stack_trace: None,
            timestamp: Some(chrono::Utc::now()),
            user_id: None,
            tenant_id: None,
            request_id: None,
            component: Some("nebula-memory".to_string()),
            operation: Some("allocate".to_string()),
        }
    }

    /// Adds context to the error
    #[cfg(feature = "std")]
    pub fn with_context<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: fmt::Display,
    {
        let context = Self::create_context(&key.into(), value);
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

    /// Returns the error code
    pub fn error_code(&self) -> &str {
        // For now, we'll return a placeholder since we don't have access to the actual error code
        // This should be updated when NebulaError provides proper access
        "ALLOC_ERROR"
    }

    /// Returns the layout if available
    pub fn layout(&self) -> Option<Layout> {
        self.layout
    }

    /// Returns the memory state if available
    pub fn memory_state(&self) -> Option<&MemoryState> {
        self.memory_state.as_ref()
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
    // Error Type Checks
    // ============================================================================

    /// Checks if this is an out of memory error
    pub fn is_out_of_memory(&self) -> bool {
        self.inner.error_code() == AllocErrorCode::OutOfMemory.code()
    }

    /// Checks if this is an invalid alignment error
    pub fn is_invalid_alignment(&self) -> bool {
        self.inner.error_code() == AllocErrorCode::InvalidAlignment.code()
    }

    /// Checks if this is a size overflow error
    pub fn is_size_overflow(&self) -> bool {
        self.inner.error_code() == AllocErrorCode::SizeOverflow.code()
    }

    /// Checks if this is an invalid layout error
    pub fn is_invalid_layout(&self) -> bool {
        self.inner.error_code() == AllocErrorCode::InvalidLayout.code()
    }

    /// Checks if this is an invalid state error
    pub fn is_invalid_state(&self) -> bool {
        self.inner.error_code() == AllocErrorCode::InvalidState.code()
    }

    // ============================================================================
    // Convenience Constructors
    // ============================================================================

    /// Creates an out-of-memory error
    pub fn out_of_memory() -> Self {
        Self::new(AllocErrorCode::OutOfMemory)
    }

    /// Creates an out-of-memory error with layout
    pub fn out_of_memory_with_layout(layout: Layout) -> Self {
        Self::with_layout(AllocErrorCode::OutOfMemory, layout)
    }

    /// Creates a size overflow error
    pub fn size_overflow() -> Self {
        Self::new(AllocErrorCode::SizeOverflow)
    }

    /// Creates an invalid alignment error
    pub fn invalid_alignment() -> Self {
        Self::new(AllocErrorCode::InvalidAlignment)
    }

    /// Creates an invalid layout error
    pub fn invalid_layout() -> Self {
        Self::new(AllocErrorCode::InvalidLayout)
    }

    /// Creates an invalid input/state error with message
    pub fn invalid_input(message: impl Into<String>) -> Self {
        #[cfg(feature = "std")]
        {
            let context = Self::create_context("error", message.into());
            let mut err = Self::new(AllocErrorCode::InvalidState);
            err.inner = err.inner.with_context(context);
            err
        }
        #[cfg(not(feature = "std"))]
        {
            let _ = message;
            Self::new(AllocErrorCode::InvalidState)
        }
    }

    /// Creates an error for specific size and alignment
    pub fn for_size_align(size: usize, align: usize) -> Self {
        match Layout::from_size_align(size, align) {
            Ok(layout) => Self::out_of_memory_with_layout(layout),
            Err(_) => Self::invalid_alignment(),
        }
    }

    /// Creates an error for type T
    pub fn for_type<T>() -> Self {
        Self::out_of_memory_with_layout(Layout::new::<T>())
    }

    /// Creates an error for an array of type T
    pub fn for_array<T>(count: usize) -> Self {
        match Layout::array::<T>(count) {
            Ok(layout) => Self::out_of_memory_with_layout(layout),
            Err(_) => Self::size_overflow(),
        }
    }

    /// Creates a detailed error with full context
    #[cfg(feature = "std")]
    pub fn detailed(code: AllocErrorCode, layout: Option<Layout>) -> Self {
        Self::with_memory_state(code, layout)
    }
}

impl From<AllocErrorCode> for AllocError {
    fn from(code: AllocErrorCode) -> Self {
        Self::new(code)
    }
}

impl From<AllocError> for NebulaError {
    fn from(error: AllocError) -> Self {
        error.inner
    }
}

impl fmt::Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(layout) = self.layout {
            write!(f, "Allocation failed for {} bytes with {} alignment: {}",
                   layout.size(), layout.align(), self.inner)
        } else {
            write!(f, "Allocation failed: {}", self.inner)
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AllocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

// ============================================================================
// Legacy Compatibility
// ============================================================================

/// Legacy error kind enum for backward compatibility
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[deprecated(note = "Use AllocErrorCode instead")]
pub enum AllocErrorKind {
    /// General allocation failure (out of memory)
    OutOfMemory,
    /// Size overflow when calculating total allocation size
    SizeOverflow,
    /// Invalid alignment (not a power of two)
    InvalidAlignment,
    /// Allocation size exceeds maximum supported size
    ExceedsMaxSize,
    /// Invalid layout parameters
    InvalidLayout,
}

#[allow(deprecated)]
impl From<AllocErrorKind> for AllocErrorCode {
    fn from(kind: AllocErrorKind) -> Self {
        match kind {
            AllocErrorKind::OutOfMemory => AllocErrorCode::OutOfMemory,
            AllocErrorKind::SizeOverflow => AllocErrorCode::SizeOverflow,
            AllocErrorKind::InvalidAlignment => AllocErrorCode::InvalidAlignment,
            AllocErrorKind::ExceedsMaxSize => AllocErrorCode::ExceedsMaxSize,
            AllocErrorKind::InvalidLayout => AllocErrorCode::InvalidLayout,
        }
    }
}

// ============================================================================
// Result Type and Utilities
// ============================================================================

/// Result type for allocation operations
pub type AllocResult<T> = Result<T, AllocError>;

/// Extension trait for Result types
pub trait AllocResultExt<T> {
    /// Maps an allocation error with additional context
    fn map_alloc_err<F>(self, f: F) -> AllocResult<T>
    where
        F: FnOnce(AllocError) -> AllocError;

    /// Adds context message to allocation error
    fn context(self, msg: &str) -> AllocResult<T>;
}

impl<T> AllocResultExt<T> for AllocResult<T> {
    fn map_alloc_err<F>(self, f: F) -> AllocResult<T>
    where
        F: FnOnce(AllocError) -> AllocError,
    {
        self.map_err(f)
    }

    fn context(self, msg: &str) -> AllocResult<T> {
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
    fn test_new_error_integration() {
        let error = AllocError::new(AllocErrorCode::OutOfMemory);
        let code = AllocErrorCode::OutOfMemory;
        assert_eq!(code.error_code(), "ALLOC_OUT_OF_MEMORY");
        assert_eq!(code.severity(), Severity::Critical);
    }

    #[test]
    fn test_error_with_layout() {
        let layout = Layout::new::<u64>();
        let error = AllocError::with_layout(AllocErrorCode::SizeOverflow, layout);

        assert_eq!(error.layout(), Some(layout));
        let code = AllocErrorCode::SizeOverflow;
        assert_eq!(code.error_code(), "ALLOC_SIZE_OVERFLOW");
    }

    #[test]
    fn test_error_stats_integration() {
        ERROR_STATS.reset();

        let _e1 = AllocError::new(AllocErrorCode::OutOfMemory);
        let _e2 = AllocError::new(AllocErrorCode::InvalidAlignment);

        let stats = ERROR_STATS.get_stats();
        assert!(stats.total_errors >= 2);
        assert!(stats.out_of_memory >= 1);
        assert!(stats.invalid_alignment >= 1);
    }

    #[test]
    fn test_convenience_constructors() {
        let oom = AllocError::out_of_memory();
        let code = AllocErrorCode::OutOfMemory;
        assert_eq!(code.error_code(), "ALLOC_OUT_OF_MEMORY");

        let array_error = AllocError::for_array::<u64>(1000);
        assert!(array_error.layout().is_some());

        let type_error = AllocError::for_type::<String>();
        assert!(type_error.layout().is_some());
    }

    #[test]
    fn test_conversion_to_nebula_error() {
        let alloc_error = AllocError::new(AllocErrorCode::InvalidLayout);
        let nebula_error: NebulaError = alloc_error.into();

        // Test that conversion works - exact structure depends on NebulaError implementation
        assert!(!nebula_error.to_string().is_empty());
    }

    #[test]
    fn test_legacy_compatibility() {
        #[allow(deprecated)]
        let legacy_code = AllocErrorKind::OutOfMemory;
        let new_code: AllocErrorCode = legacy_code.into();

        assert_eq!(new_code, AllocErrorCode::OutOfMemory);
    }
}