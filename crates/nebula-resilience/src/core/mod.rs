//! Core types and traits for the resilience library
//!
//! This module provides the fundamental building blocks used throughout
//! the library, including error types, traits, metrics, and configuration.
//!
//! # Module Overview
//!
//! - [`advanced`] - Advanced type system features (typestate, GADTs, variance)
//! - [`category`] - Sealed category traits for pattern classification
//! - [`config`] - Configuration management and validation
//! - [`traits`] - Core resilience traits and abstractions
//! - [`types`] - Type-safe newtypes for configuration values

pub mod advanced;
pub mod cancellation;
pub mod categories;
pub mod config;
pub mod dynamic;
mod error;
mod metrics;
mod result;
pub mod traits;
pub mod types;

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
pub use dynamic::{
    BulkheadConfigBuilder, CircuitBreakerConfigBuilder, DynamicConfig, DynamicConfigBuilder,
    DynamicConfigurable, ResiliencePresets, RetryConfigBuilder,
};
pub use error::{ErrorClass, ErrorContext, ResilienceError};
pub use metrics::{MetricKind, MetricSnapshot, Metrics, MetricsCollector};
pub use result::{ResilienceResult, ResultExt};
pub use traits::{
    Executable, HealthCheck, PatternMetrics, ResiliencePattern, Retryable,
    circuit_states::{Closed, HalfOpen, Open, StateTransition, TypestateCircuitState},
};

// Re-export advanced type system features
pub use advanced::{
    Aggressive, Balanced, Complete, ComposedPolicy, Conservative, ConstValidated, PolicyBuilder,
    Strategy, StrategyConfig, Unconfigured, ValidatedRetryConfig, WithCircuitBreaker, WithRetry,
};

// Re-export type-safe newtypes
pub use types::{
    DurationExt, FailureThreshold, MaxConcurrency, RateLimit, ResilienceResultExt, RetryCount,
    Timeout as TimeoutNewtype,
};

// Re-export cancellation support
pub use cancellation::{
    CancellableFuture, CancellationContext, CancellationExt, ShutdownCoordinator,
};

// Re-export unified categories
pub use categories::{
    Category, PatternCategory, ServiceCategory,
    pattern::{Fallback, FlowControl, Protection, Retry, Timeout as TimeoutCategory},
    service::{Cache, Database, Generic, Http, MessageQueue},
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
