//! Error types for the metrics subsystem.

/// Primitive metric kind stored in a
/// [`nebula_metrics::MetricsRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Monotonic counter.
    Counter,
    /// Signed gauge.
    Gauge,
    /// Histogram with fixed bucket layout.
    Histogram,
}

/// Errors that can occur in the metrics subsystem.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum MetricsError {
    /// An I/O error occurred in a sink.
    #[classify(category = "internal", code = "METRICS:IO")]
    #[error("sink I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The same `(name, labels)` identity is already registered with a different primitive kind.
    #[classify(
        category = "validation",
        code = "METRICS:METRIC_KIND_CONFLICT",
        retryable = false
    )]
    #[error(
        "metric `{metric_name}` is registered as {actual_kind:?} but {expected_kind:?} was requested"
    )]
    MetricKindConflict {
        /// Human-readable metric name (resolved from the interner).
        metric_name: String,
        /// Kind the caller requested.
        expected_kind: MetricKind,
        /// Kind already stored for this identity.
        actual_kind: MetricKind,
    },

    /// A histogram series already exists with different finite bucket boundaries.
    #[classify(
        category = "validation",
        code = "METRICS:HISTOGRAM_LAYOUT_CONFLICT",
        retryable = false
    )]
    #[error(
        "histogram `{metric_name}` already exists with incompatible bucket boundaries (layout is pinned at first registration)"
    )]
    HistogramLayoutConflict {
        /// Human-readable metric name.
        metric_name: String,
    },

    /// Histogram bucket configuration is invalid for a primitive histogram.
    #[classify(
        category = "validation",
        code = "METRICS:INVALID_HISTOGRAM_BUCKETS",
        retryable = false
    )]
    #[error("invalid histogram bucket boundaries: {reason}")]
    InvalidHistogramBuckets {
        /// Why the boundaries were rejected.
        reason: String,
    },
}

/// Type alias for results in the metrics subsystem.
pub type MetricsResult<T> = Result<T, MetricsError>;
