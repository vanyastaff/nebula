//! Core types and traits for the resilience library
//!
//! This module provides the fundamental building blocks used throughout
//! the library, including error types, traits, metrics, and configuration.

mod error;
mod result;
mod traits;
mod metrics;
mod config;
mod dynamic;

// Re-export primary types
pub use error::{ResilienceError, ErrorClass, ErrorContext};
pub use result::{ResilienceResult, ResultExt, AsyncResultExt, ErrorCollector};
pub use traits::{
    ResiliencePattern,
    Executable,
    Retryable,
    PatternMetrics,
    HealthCheck,
    CircuitState,
};
pub use metrics::{
    Metrics,
    MetricsCollector,
    MetricSnapshot,
    MetricKind,
};
pub use config::{
    ResilienceConfig,
    CommonConfig,
    Configurable,
    ResilienceConfigManager,
    // Re-export nebula-config types
    NebulaConfig,
    ConfigBuilder,
    ConfigSource,
    ConfigResult,
    ConfigError,
};
pub use dynamic::{
    DynamicConfig,
    DynamicConfigurable,
    ResiliencePresets,
};

/// Core constants
pub mod constants {
    use std::time::Duration;

    /// Default timeout duration
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Default retry attempts
    pub const DEFAULT_RETRY_ATTEMPTS: usize = 3;

    /// Default circuit breaker threshold
    pub const DEFAULT_FAILURE_THRESHOLD: usize = 5;

    /// Default rate limit
    pub const DEFAULT_RATE_LIMIT: f64 = 100.0;
}