//! Severity level of a validation error.

/// Severity level of a validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ErrorSeverity {
    /// Error that must be fixed (default).
    #[default]
    Error,
    /// Warning that should be addressed but doesn't block validation.
    Warning,
    /// Informational message.
    Info,
}
