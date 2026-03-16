//! Error types for the telemetry subsystem.

/// Errors that can occur in the telemetry subsystem.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// The metric name is invalid.
    ///
    /// Names must be non-empty and should follow the `nebula_*` naming convention.
    #[error("invalid metric name: {name:?}")]
    InvalidMetricName {
        /// The rejected name.
        name: String,
    },

    /// A label key is invalid.
    ///
    /// Keys must be non-empty ASCII strings.
    #[error("invalid label key: {key:?}")]
    InvalidLabelKey {
        /// The rejected key.
        key: String,
    },

    /// A label value is invalid.
    #[error("invalid label value: {value:?}")]
    InvalidLabelValue {
        /// The rejected value.
        value: String,
    },

    /// The recorder channel is closed (background task was dropped).
    #[error("telemetry recorder channel is closed")]
    RecorderClosed,

    /// An I/O error occurred in a sink.
    #[error("sink I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl TelemetryError {
    /// Convenience constructor for invalid metric name.
    pub fn invalid_metric_name(name: impl Into<String>) -> Self {
        Self::InvalidMetricName { name: name.into() }
    }

    /// Convenience constructor for invalid label key.
    pub fn invalid_label_key(key: impl Into<String>) -> Self {
        Self::InvalidLabelKey { key: key.into() }
    }

    /// Convenience constructor for invalid label value.
    pub fn invalid_label_value(value: impl Into<String>) -> Self {
        Self::InvalidLabelValue {
            value: value.into(),
        }
    }

    /// Whether this error is transient and the operation could be retried.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::RecorderClosed)
    }
}

/// Type alias for results in the telemetry subsystem.
pub type TelemetryResult<T> = Result<T, TelemetryError>;
