//! Core types and traits for the resilience library
//!
//! This module provides the fundamental building blocks used throughout
//! the library, including error types, traits, metrics, and configuration.

pub mod config;
mod dynamic;
mod error;
mod metrics;
mod result;
mod traits;

// Re-export primary types
pub use config::{
    CommonConfig,
    ConfigBuilder,
    ConfigError,
    ConfigResult,
    ConfigSource,
    Configurable,
    // Re-export nebula-config types
    NebulaConfig,
    ResilienceConfig,
    ResilienceConfigManager,
};
pub use dynamic::{DynamicConfig, DynamicConfigurable, ResiliencePresets};
pub use error::{ErrorClass, ErrorContext, ResilienceError};
pub use metrics::{MetricKind, MetricSnapshot, Metrics, MetricsCollector};
pub use result::{AsyncResultExt, ErrorCollector, ResilienceResult, ResultExt};
pub use traits::{
    CircuitState, Executable, HealthCheck, PatternMetrics, ResiliencePattern, Retryable,
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
