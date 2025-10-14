//! Resilience patterns for building fault-tolerant systems
//!
//! This crate provides resilience patterns including retry, circuit breaker,
//! bulkhead, rate limiter, and timeout functionality.

#![allow(clippy::module_name_repetitions)]
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]

mod compose;
pub mod core;
pub mod helpers;
mod manager;
pub mod observability;
pub mod patterns;
mod policy;
pub mod retryable;

// Patterns module

// Observability

// High-level abstractions

// Re-exports from core
pub use core::{
    BulkheadConfigBuilder,
    CircuitBreakerConfigBuilder,
    ConfigError,
    ConfigResult,
    DynamicConfig,
    DynamicConfigBuilder,
    DynamicConfigurable,
    ErrorClass,
    Executable,
    // Configuration types
    ResilienceConfig,
    ResilienceConfigManager,
    ResilienceError,
    ResiliencePattern,
    ResiliencePresets,
    ResilienceResult,
    // Builder types
    RetryConfigBuilder,
};

// Re-exports from patterns
pub use patterns::{
    AdaptiveRateLimiter,
    LeakyBucket,
    SlidingWindow,
    TokenBucket,
    // Basic patterns
    bulkhead::{Bulkhead, BulkheadConfig},
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState},
    fallback::{AnyStringFallbackStrategy, FallbackStrategy, ValueFallback},
    hedge::{HedgeConfig, HedgeExecutor},

    // Rate limiting
    rate_limiter::{AnyRateLimiter, RateLimiter},
    retry::{RetryStrategy, retry},
    timeout::{timeout, timeout_with_original_error},
};

// Re-export high-level abstractions
pub use compose::{LayerBuilder, ResilienceChain, ResilienceLayer};
pub use manager::{PolicyBuilder, ResilienceManager, RetryableOperation, ServiceMetrics};
pub use policy::{PolicyMetadata, ResiliencePolicy};

// Re-export Retryable trait
pub use retryable::Retryable;

/// Prelude
pub mod prelude {
    // Core types
    pub use crate::core::{ResilienceError, ResilienceResult};

    // Pattern primitives
    pub use crate::patterns::{
        Bulkhead, BulkheadConfig, CircuitBreaker, CircuitBreakerConfig, CircuitState,
        RetryStrategy, timeout,
    };

    // High-level abstractions
    pub use crate::{ResilienceChain, ResilienceManager, ResiliencePolicy};

    // Configuration
    pub use crate::{
        ConfigError, ConfigResult, DynamicConfig, DynamicConfigBuilder, ResilienceConfig,
        ResilienceConfigManager, ResiliencePresets,
    };

    // Re-export Retryable trait
    pub use crate::Retryable;

    // Re-export nebula ecosystem for convenience
    pub use nebula_config::ConfigSource;
    pub use nebula_log::{debug, error, info, warn};
    pub use nebula_value::Value;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
