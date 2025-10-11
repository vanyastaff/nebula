#![allow(clippy::module_name_repetitions)]
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
// Core module
//! # Nebula Resilience
pub mod core;
pub mod helpers;

// Patterns module
pub mod patterns;

// High-level abstractions
mod compose;
mod manager;
mod policy;

// Re-exports from core
pub use core::{
    ConfigError,
    ConfigResult,
    DynamicConfig,
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
        ConfigError, ConfigResult, DynamicConfig, ResilienceConfig, ResilienceConfigManager,
        ResiliencePresets,
    };

    // Re-export nebula ecosystem for convenience
    pub use nebula_config::ConfigSource;
    pub use nebula_log::{debug, error, info, warn};
    pub use nebula_value::Value;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
