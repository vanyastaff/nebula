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

impl nebula_error::Classify for TelemetryError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::RecorderClosed => nebula_error::ErrorCategory::Internal,
            Self::Io(_) => nebula_error::ErrorCategory::Internal,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::RecorderClosed => "TELEMETRY:RECORDER_CLOSED",
            Self::Io(_) => "TELEMETRY:IO",
        })
    }
}

/// Type alias for results in the telemetry subsystem.
pub type TelemetryResult<T> = Result<T, TelemetryError>;
