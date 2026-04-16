//! Semantic field and event naming conventions
//!
//! Provides compile-time constants for field names used in observability contexts,
//! events, and tracing spans. Using these constants prevents typos and enables
//! IDE-assisted refactoring.

// ---------------------------------------------------------------------------
// Context field names
// ---------------------------------------------------------------------------

/// Field names for execution context spans and events.
pub mod field {
    /// Unique execution ID
    pub const EXECUTION_ID: &str = "execution_id";
    /// Workflow definition ID
    pub const WORKFLOW_ID: &str = "workflow_id";
    /// Tenant ID for multi-tenancy
    pub const TENANT_ID: &str = "tenant_id";
    /// Parent execution ID (for sub-workflows)
    pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
    /// Distributed trace ID
    pub const TRACE_ID: &str = "trace_id";
    /// Node instance ID
    pub const NODE_ID: &str = "node_key";
    /// Action type ID (e.g., "http.request")
    pub const ACTION_ID: &str = "action_id";
    /// Retry attempt count
    pub const RETRY_COUNT: &str = "retry_count";

    /// Operation name (for lifecycle events)
    pub const OPERATION: &str = "operation";
    /// Duration in milliseconds
    pub const DURATION_MS: &str = "duration_ms";
    /// Duration in fractional seconds
    pub const DURATION_SECS: &str = "duration_secs";
    /// Error message
    pub const ERROR: &str = "error";
    /// Additional context string
    pub const CONTEXT: &str = "context";

    /// Service name (global field / OTel resource)
    pub const SERVICE: &str = "service";
    /// Environment (dev/staging/prod)
    pub const ENV: &str = "env";
    /// Application version
    pub const VERSION: &str = "version";
    /// Instance ID
    pub const INSTANCE: &str = "instance";
    /// Deployment region
    pub const REGION: &str = "region";
}

// ---------------------------------------------------------------------------
// Typed event kinds
// ---------------------------------------------------------------------------

/// Typed event identifier.
///
/// Provides compile-time safety for event names instead of arbitrary strings.
/// New variants can be added without breaking existing hooks (the enum is `#[non_exhaustive]`).
///
/// Hooks can match on `EventKind` instead of doing string comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventKind {
    /// An operation has started
    OperationStarted,
    /// An operation completed successfully
    OperationCompleted,
    /// An operation failed
    OperationFailed,
}

impl EventKind {
    /// Get the canonical string name for this event kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OperationStarted => "operation_started",
            Self::OperationCompleted => "operation_completed",
            Self::OperationFailed => "operation_failed",
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
