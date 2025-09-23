//! # Nebula Resilience

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

// Core module
pub mod core;

// Patterns module
pub mod patterns;

// High-level abstractions
mod compose;
mod manager;
mod policy;

// Re-exports from core
pub use core::{
    ResilienceError,
    ResilienceResult,
    ErrorClass,
    ResiliencePattern,
    Executable,
    // Configuration types
    ResilienceConfig,
    ResilienceConfigManager,
    DynamicConfig,
    DynamicConfigurable,
    ResiliencePresets,
    ConfigResult,
    ConfigError,
};

// Re-exports from patterns
pub use patterns::{
    // Basic patterns
    bulkhead::{Bulkhead, BulkheadConfig},
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState},
    retry::{retry, RetryStrategy},
    timeout::{timeout, timeout_with_original_error},
    fallback::{FallbackStrategy, ValueFallback, AnyStringFallbackStrategy},
    hedge::{HedgeExecutor, HedgeConfig},

    // Rate limiting
    rate_limiter::{RateLimiter, AnyRateLimiter},
    TokenBucket,
    LeakyBucket,
    SlidingWindow,
    AdaptiveRateLimiter,
};

// Re-export high-level abstractions
pub use compose::{ResilienceChain, LayerBuilder, ResilienceLayer};
pub use policy::{ResiliencePolicy, PolicyMetadata};
pub use manager::{ResilienceManager, PolicyBuilder, RetryableOperation};

/// Prelude
pub mod prelude {
    pub use crate::core::{ResilienceError, ResilienceResult};
    pub use crate::patterns::{Bulkhead, CircuitBreaker, RetryStrategy, timeout};
    pub use crate::{
        ResiliencePolicy,
        ResilienceManager,
        ResilienceChain,
        // Configuration
        ResilienceConfig,
        DynamicConfig,
        ResiliencePresets,
        ConfigResult,
    };

    // Re-export nebula ecosystem for convenience
    pub use nebula_log::{debug, info, warn, error};
    pub use nebula_value::Value;
    pub use nebula_config::ConfigSource;
}


/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");