//! Error types for the telemetry subsystem.

/// Errors that can occur in the telemetry subsystem.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// The recorder channel is closed (background task was dropped).
    #[error("telemetry recorder channel is closed")]
    RecorderClosed,

    /// An I/O error occurred in a sink.
    #[error("sink I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Type alias for results in the telemetry subsystem.
pub type TelemetryResult<T> = Result<T, TelemetryError>;
