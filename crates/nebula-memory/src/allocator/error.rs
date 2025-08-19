//! Enhanced memory allocation error type with comprehensive diagnostics
//!
//! Provides a unified error type for memory allocation operations with:
//! - Cross-platform support (stable and nightly)
//! - Rich context and debugging information
//! - Error chains and source tracking
//! - Optional backtrace capture
//! - Telemetry and metrics integration

use core::alloc::Layout;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "std")]
use std::backtrace::{Backtrace, BacktraceStatus};

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
            total_errors: AtomicU64::new(0),
        }
    }

    fn record(&self, kind: AllocErrorKind) {
        self.total_errors.fetch_add(1, Ordering::Relaxed);
        match kind {
            AllocErrorKind::OutOfMemory => {
                self.out_of_memory.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorKind::SizeOverflow => {
                self.size_overflow.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorKind::InvalidAlignment => {
                self.invalid_alignment.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorKind::ExceedsMaxSize => {
                self.exceeds_max_size.fetch_add(1, Ordering::Relaxed);
            }
            AllocErrorKind::InvalidLayout => {
                self.invalid_layout.fetch_add(1, Ordering::Relaxed);
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
            total_errors: self.total_errors.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.out_of_memory.store(0, Ordering::Relaxed);
        self.size_overflow.store(0, Ordering::Relaxed);
        self.invalid_alignment.store(0, Ordering::Relaxed);
        self.exceeds_max_size.store(0, Ordering::Relaxed);
        self.invalid_layout.store(0, Ordering::Relaxed);
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
    pub total_errors: u64,
}

/// Global error statistics instance
pub static ERROR_STATS: ErrorStats = ErrorStats::new();

// ============================================================================
// Error Context
// ============================================================================

/// Additional context information for allocation errors
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Optional message providing additional context
    pub message: Option<&'static str>,
    /// The allocation site location
    pub location: Option<&'static core::panic::Location<'static>>,
    /// Thread ID where the error occurred (if available)
    #[cfg(feature = "std")]
    pub thread_id: Option<std::thread::ThreadId>,
    /// Timestamp when the error occurred (if available)
    #[cfg(feature = "std")]
    pub timestamp: Option<std::time::SystemTime>,
    /// System memory state at time of error
    pub memory_state: Option<MemoryState>,
}

impl ErrorContext {
    pub const fn new() -> Self {
        Self {
            message: None,
            location: None,
            #[cfg(feature = "std")]
            thread_id: None,
            #[cfg(feature = "std")]
            timestamp: None,
            memory_state: None,
        }
    }

    #[track_caller]
    pub fn with_caller() -> Self {
        Self {
            message: None,
            location: Some(core::panic::Location::caller()),
            #[cfg(feature = "std")]
            thread_id: Some(std::thread::current().id()),
            #[cfg(feature = "std")]
            timestamp: Some(std::time::SystemTime::now()),
            memory_state: None,
        }
    }

    pub fn with_message(mut self, message: &'static str) -> Self {
        self.message = Some(message);
        self
    }

    pub fn with_memory_state(mut self, state: MemoryState) -> Self {
        self.memory_state = Some(state);
        self
    }
}

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
    pub fn capture() -> Self {
        // This would integrate with system memory APIs
        // For now, returning a placeholder
        Self::new()
    }
}

// ============================================================================
// Enhanced Error Types
// ============================================================================

/// Extended allocation error kinds with detailed categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl AllocErrorKind {
    /// Returns a static string describing the error
    pub const fn as_str(&self) -> &'static str {
        match self {
            AllocErrorKind::OutOfMemory => "out of memory",
            AllocErrorKind::SizeOverflow => "size overflow",
            AllocErrorKind::InvalidAlignment => "invalid alignment",
            AllocErrorKind::ExceedsMaxSize => "exceeds maximum allocation size",
            AllocErrorKind::InvalidLayout => "invalid layout",
        }
    }

    /// Returns the error severity level
    pub const fn severity(&self) -> ErrorSeverity {
        match self {
            AllocErrorKind::OutOfMemory => ErrorSeverity::Critical,
            AllocErrorKind::SizeOverflow => ErrorSeverity::Error,
            AllocErrorKind::InvalidAlignment => ErrorSeverity::Error,
            AllocErrorKind::ExceedsMaxSize => ErrorSeverity::Warning,
            AllocErrorKind::InvalidLayout => ErrorSeverity::Error,
        }
    }

    /// Returns suggested recovery action
    pub const fn recovery_hint(&self) -> &'static str {
        match self {
            AllocErrorKind::OutOfMemory => {
                "Try freeing memory or increasing system resources"
            }
            AllocErrorKind::SizeOverflow => {
                "Reduce allocation size or split into smaller allocations"
            }
            AllocErrorKind::InvalidAlignment => {
                "Ensure alignment is a power of two"
            }
            AllocErrorKind::ExceedsMaxSize => {
                "Split allocation into smaller chunks"
            }
            AllocErrorKind::InvalidLayout => {
                "Check layout parameters for validity"
            }
        }
    }
}

impl fmt::Display for AllocErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    Warning,
    Error,
    Critical,
}

// ============================================================================
// Main Error Type (Stable Version)
// ============================================================================

#[cfg(not(feature = "nightly"))]
#[derive(Debug, Clone)]
pub struct AllocError {
    /// The specific kind of allocation error
    kind: AllocErrorKind,
    /// Optional information about the layout that failed to allocate
    layout: Option<Layout>,
    /// Additional context information
    context: Option<Box<ErrorContext>>,
    /// Optional error source for error chains
    #[cfg(feature = "std")]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    /// Optional backtrace
    #[cfg(feature = "std")]
    backtrace: Option<Backtrace>,
}

#[cfg(not(feature = "nightly"))]
impl AllocError {
    /// Creates a new allocation error without additional information
    #[inline]
    pub const fn new() -> Self {
        ERROR_STATS.record(AllocErrorKind::OutOfMemory);
        Self {
            kind: AllocErrorKind::OutOfMemory,
            layout: None,
            context: None,
            #[cfg(feature = "std")]
            source: None,
            #[cfg(feature = "std")]
            backtrace: None,
        }
    }

    /// Creates a new allocation error with a specific kind
    #[inline]
    pub fn new_with_kind(kind: AllocErrorKind) -> Self {
        ERROR_STATS.record(kind);
        Self {
            kind,
            layout: None,
            context: None,
            #[cfg(feature = "std")]
            source: None,
            #[cfg(feature = "std")]
            backtrace: capture_backtrace(),
        }
    }

    /// Creates a new allocation error with layout information
    #[inline]
    pub fn with_layout(layout: Layout) -> Self {
        ERROR_STATS.record(AllocErrorKind::OutOfMemory);
        Self {
            kind: AllocErrorKind::OutOfMemory,
            layout: Some(layout),
            context: None,
            #[cfg(feature = "std")]
            source: None,
            #[cfg(feature = "std")]
            backtrace: capture_backtrace(),
        }
    }

    /// Creates a new allocation error with both kind and layout information
    #[inline]
    #[track_caller]
    pub fn with_kind_and_layout(kind: AllocErrorKind, layout: Layout) -> Self {
        ERROR_STATS.record(kind);
        Self {
            kind,
            layout: Some(layout),
            context: Some(Box::new(ErrorContext::with_caller())),
            #[cfg(feature = "std")]
            source: None,
            #[cfg(feature = "std")]
            backtrace: capture_backtrace(),
        }
    }

    /// Adds context to the error
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(Box::new(context));
        self
    }

    /// Adds a message to the error
    pub fn with_message(mut self, message: &'static str) -> Self {
        let context = self.context.take().map(|c| *c).unwrap_or_else(ErrorContext::new);
        self.context = Some(Box::new(context.with_message(message)));
        self
    }

    /// Adds source error for error chaining
    #[cfg(feature = "std")]
    pub fn with_source<E>(mut self, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.source = Some(Box::new(source));
        self
    }

    /// Returns the specific error kind
    #[inline]
    pub const fn kind(&self) -> AllocErrorKind {
        self.kind
    }

    /// Returns the layout associated with this error, if any
    #[inline]
    pub const fn layout(&self) -> Option<Layout> {
        self.layout
    }

    /// Returns the error context
    #[inline]
    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_deref()
    }

    /// Returns the error severity
    #[inline]
    pub const fn severity(&self) -> ErrorSeverity {
        self.kind.severity()
    }

    /// Returns recovery hint
    #[inline]
    pub const fn recovery_hint(&self) -> &'static str {
        self.kind.recovery_hint()
    }

    /// Checks if the error contains layout information
    #[inline]
    pub const fn has_layout(&self) -> bool {
        self.layout.is_some()
    }

    /// Checks if this is an out-of-memory error
    #[inline]
    pub const fn is_out_of_memory(&self) -> bool {
        matches!(self.kind, AllocErrorKind::OutOfMemory)
    }

    /// Checks if this is a size overflow error
    #[inline]
    pub const fn is_size_overflow(&self) -> bool {
        matches!(self.kind, AllocErrorKind::SizeOverflow)
    }

    /// Checks if this is an invalid alignment error
    #[inline]
    pub const fn is_invalid_alignment(&self) -> bool {
        matches!(self.kind, AllocErrorKind::InvalidAlignment)
    }

    /// Checks if this is a critical error
    #[inline]
    pub const fn is_critical(&self) -> bool {
        matches!(self.severity(), ErrorSeverity::Critical)
    }

    /// Returns backtrace if available
    #[cfg(feature = "std")]
    pub fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }

    /// Formats the error with full details
    pub fn detailed_format(&self) -> String {
        let mut output = format!("AllocError: {}\n", self.kind);

        if let Some(layout) = self.layout {
            output.push_str(&format!(
                "  Layout: {} bytes, {} alignment\n",
                layout.size(),
                layout.align()
            ));
        }

        if let Some(context) = &self.context {
            if let Some(msg) = context.message {
                output.push_str(&format!("  Message: {}\n", msg));
            }

            if let Some(loc) = context.location {
                output.push_str(&format!("  Location: {}\n", loc));
            }

            #[cfg(feature = "std")]
            if let Some(thread_id) = context.thread_id {
                output.push_str(&format!("  Thread: {:?}\n", thread_id));
            }

            if let Some(mem_state) = &context.memory_state {
                output.push_str("  Memory State:\n");
                if let Some(avail) = mem_state.available {
                    output.push_str(&format!("    Available: {} bytes\n", avail));
                }
                if let Some(total) = mem_state.total {
                    output.push_str(&format!("    Total: {} bytes\n", total));
                }
            }
        }

        output.push_str(&format!("  Severity: {:?}\n", self.severity()));
        output.push_str(&format!("  Recovery: {}\n", self.recovery_hint()));

        #[cfg(feature = "std")]
        if let Some(bt) = &self.backtrace {
            if bt.status() == BacktraceStatus::Captured {
                output.push_str(&format!("  Backtrace:\n{:?}\n", bt));
            }
        }

        output
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

#[cfg(feature = "std")]
fn capture_backtrace() -> Option<Backtrace> {
    let bt = Backtrace::capture();
    if bt.status() == BacktraceStatus::Captured {
        Some(bt)
    } else {
        None
    }
}

// ============================================================================
// Trait Implementations
// ============================================================================

#[cfg(not(feature = "nightly"))]
impl Default for AllocError {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "nightly"))]
impl fmt::Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.layout {
            Some(layout) => write!(
                f,
                "memory allocation failed ({}): could not allocate {} bytes with alignment {}",
                self.kind,
                layout.size(),
                layout.align()
            ),
            None => write!(f, "memory allocation failed ({})", self.kind),
        }
    }
}

#[cfg(all(not(feature = "nightly"), feature = "std"))]
impl std::error::Error for AllocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }

    #[cfg(feature = "std")]
    fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }
}

// ============================================================================
// Nightly Support (simplified for brevity)
// ============================================================================

#[cfg(feature = "nightly")]
pub use self::nightly::AllocError;

#[cfg(feature = "nightly")]
mod nightly {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct AllocError {
        inner: core::alloc::AllocError,
        kind: AllocErrorKind,
        layout: Option<Layout>,
        context: Option<Box<ErrorContext>>,
        #[cfg(feature = "std")]
        backtrace: Option<Backtrace>,
    }

    // Implementation would be similar to stable version
    // but wrapping core::alloc::AllocError
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
    fn context(self, msg: &'static str) -> AllocResult<T>;
}

impl<T> AllocResultExt<T> for AllocResult<T> {
    fn map_alloc_err<F>(self, f: F) -> AllocResult<T>
    where
        F: FnOnce(AllocError) -> AllocError,
    {
        self.map_err(f)
    }

    fn context(self, msg: &'static str) -> AllocResult<T> {
        self.map_err(|e| e.with_message(msg))
    }
}

// ============================================================================
// Error Builder Pattern
// ============================================================================

/// Builder for constructing detailed allocation errors
pub struct AllocErrorBuilder {
    kind: AllocErrorKind,
    layout: Option<Layout>,
    message: Option<&'static str>,
    memory_state: Option<MemoryState>,
}

impl AllocErrorBuilder {
    pub fn new(kind: AllocErrorKind) -> Self {
        Self {
            kind,
            layout: None,
            message: None,
            memory_state: None,
        }
    }

    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = Some(layout);
        self
    }

    pub fn message(mut self, message: &'static str) -> Self {
        self.message = Some(message);
        self
    }

    pub fn memory_state(mut self, state: MemoryState) -> Self {
        self.memory_state = Some(state);
        self
    }

    #[track_caller]
    pub fn build(self) -> AllocError {
        let mut error = if let Some(layout) = self.layout {
            AllocError::with_kind_and_layout(self.kind, layout)
        } else {
            AllocError::new_with_kind(self.kind)
        };

        let mut context = ErrorContext::with_caller();

        if let Some(msg) = self.message {
            context = context.with_message(msg);
        }

        if let Some(state) = self.memory_state {
            context = context.with_memory_state(state);
        }

        error.with_context(context)
    }
}

// ============================================================================
// Convenience Constructors
// ============================================================================

impl AllocError {
    /// Creates an allocation error for specific size and alignment
    #[inline]
    pub fn for_size_align(size: usize, align: usize) -> Self {
        match Layout::from_size_align(size, align) {
            Ok(layout) => Self::with_layout(layout),
            Err(_) => Self::new_with_kind(AllocErrorKind::InvalidAlignment),
        }
    }

    /// Creates an allocation error for type T
    #[inline]
    pub fn for_type<T>() -> Self {
        Self::with_layout(Layout::new::<T>())
    }

    /// Creates an allocation error for an array of type T
    #[inline]
    pub fn for_array<T>(count: usize) -> Self {
        match Layout::array::<T>(count) {
            Ok(layout) => Self::with_layout(layout),
            Err(_) => Self::new_with_kind(AllocErrorKind::SizeOverflow),
        }
    }

    /// Creates a detailed out-of-memory error
    #[cfg(feature = "std")]
    #[track_caller]
    pub fn out_of_memory_detailed(layout: Layout) -> Self {
        Self::with_kind_and_layout(AllocErrorKind::OutOfMemory, layout)
            .with_context(
                ErrorContext::with_caller()
                    .with_memory_state(MemoryState::capture())
            )
    }
}

// ============================================================================
// Error Recovery Helpers
// ============================================================================

/// Trait for types that can attempt recovery from allocation errors
pub trait TryRecover {
    /// Attempts to recover from the allocation error
    fn try_recover(&self, error: &AllocError) -> Option<RecoveryAction>;
}

/// Suggested recovery actions
#[derive(Debug, Clone, Copy)]
pub enum RecoveryAction {
    /// Retry the allocation after delay
    RetryAfter(core::time::Duration),
    /// Try with reduced size
    ReduceSize(usize),
    /// Free memory and retry
    FreeMemoryAndRetry,
    /// Split into smaller allocations
    SplitAllocation,
    /// Fail permanently
    Fail,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_builder() {
        let error = AllocErrorBuilder::new(AllocErrorKind::SizeOverflow)
            .layout(Layout::new::<u64>())
            .message("Test allocation failed")
            .build();

        assert_eq!(error.kind(), AllocErrorKind::SizeOverflow);
        assert!(error.has_layout());
        assert!(error.context().is_some());
    }

    #[test]
    fn test_error_stats() {
        ERROR_STATS.reset();

        let _e1 = AllocError::new_with_kind(AllocErrorKind::OutOfMemory);
        let _e2 = AllocError::new_with_kind(AllocErrorKind::SizeOverflow);

        let stats = ERROR_STATS.get_stats();
        assert!(stats.total_errors >= 2);
    }

    #[test]
    fn test_detailed_format() {
        let error = AllocError::for_array::<u64>(1000)
            .with_message("Large array allocation");

        let details = error.detailed_format();
        assert!(details.contains("AllocError"));
        assert!(details.contains("Recovery"));
    }

    #[test]
    fn test_error_severity() {
        let oom = AllocError::new_with_kind(AllocErrorKind::OutOfMemory);
        assert!(oom.is_critical());

        let overflow = AllocError::new_with_kind(AllocErrorKind::SizeOverflow);
        assert!(!overflow.is_critical());
    }
}