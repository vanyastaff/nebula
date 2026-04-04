//! Error types for the telemetry subsystem.

/// Errors that can occur in the telemetry subsystem.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum TelemetryError {
    /// An I/O error occurred in a sink.
    #[classify(category = "internal", code = "TELEMETRY:IO")]
    #[error("sink I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Type alias for results in the telemetry subsystem.
pub type TelemetryResult<T> = Result<T, TelemetryError>;
