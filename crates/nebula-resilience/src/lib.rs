#![allow(clippy::module_name_repetitions)]
#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
//! # Nebula Resilience
//!
//! A comprehensive resilience library providing essential patterns for building reliable
//! distributed systems and fault-tolerant applications.
//!
//! ## Features
//!
//! - **Circuit Breaker**: Prevent cascading failures by failing fast when a service is unhealthy
//! - **Retry**: Automatic retry with configurable backoff strategies
//! - **Timeout**: Enforce time limits on operations
//! - **Bulkhead**: Isolate resources to prevent resource exhaustion
//! - **Rate Limiting**: Control request rates with multiple algorithms
//! - **Fallback**: Provide alternative responses when operations fail
//! - **Hedge**: Send duplicate requests to reduce tail latency
//!
//! ## Quick Start
//!
//! ```no_run
//! use nebula_resilience::prelude::*;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a resilience manager
//!     let manager = ResilienceManager::new();
//!
//!     // Register a policy for a service
//!     let policy = ResiliencePolicy::default()
//!         .with_timeout(Duration::from_secs(5))
//!         .with_retry(3);
//!
//!     manager.register_policy("my-api", policy).await;
//!
//!     // Execute an operation with resilience
//!     let result = manager.execute("my-api", || async {
//!         // Your async operation here
//!         Ok(42)
//!     }).await;
//! }
//! ```
//!
//! ## Pattern Examples
//!
//! ### Circuit Breaker
//!
//! ```no_run
//! use nebula_resilience::{CircuitBreaker, CircuitBreakerConfig, ResilienceError};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = CircuitBreakerConfig::new(
//!         Duration::from_secs(60),  // reset timeout
//!         5,                         // failure threshold
//!     );
//!
//!     let cb = CircuitBreaker::new(config);
//!
//!     let result = cb.execute(|| async {
//!         // Protected operation
//!         Ok::<_, ResilienceError>("success")
//!     }).await;
//! }
//! ```
//!
//! ### Retry with Exponential Backoff
//!
//! ```no_run
//! use nebula_resilience::{retry, RetryStrategy, ResilienceError};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     let strategy = RetryStrategy::exponential_backoff(
//!         Duration::from_millis(100),  // base delay
//!         2.0,                          // multiplier
//!         3,                            // max attempts
//!     );
//!
//!     let result = retry(strategy, || async {
//!         // Operation that might fail
//!         Ok::<_, ResilienceError>(42)
//!     }).await;
//! }
//! ```
//!
//! ### Rate Limiting
//!
//! ```no_run
//! use nebula_resilience::{TokenBucket, ResilienceError};
//!
//! #[tokio::main]
//! async fn main() {
//!     // 100 requests per second, burst of 10
//!     let limiter = TokenBucket::new(100, 10.0);
//!
//!     let result = limiter.execute(|| async {
//!         // Rate-limited operation
//!         Ok::<_, ResilienceError>("processed")
//!     }).await;
//! }
//! ```
//!
//! ## Modules
//!
//! - [`core`] - Core types, errors, and configuration
//! - [`patterns`] - Individual resilience pattern implementations
//! - [`prelude`] - Common imports for convenience
//!
// Core module
pub mod core;
pub mod helpers;

// Patterns module
pub mod patterns;

// Observability
pub mod observability;

// High-level abstractions
mod compose;
mod manager;
mod policy;

// Re-exports from core
pub use core::{
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
    CircuitBreakerConfigBuilder,
    BulkheadConfigBuilder,
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
        ConfigError, ConfigResult, DynamicConfig, DynamicConfigBuilder, ResilienceConfig,
        ResilienceConfigManager, ResiliencePresets,
    };

    // Re-export nebula ecosystem for convenience
    pub use nebula_config::ConfigSource;
    pub use nebula_log::{debug, error, info, warn};
    pub use nebula_value::Value;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
