//! # Nebula Resilience
//!
//! A comprehensive resilience library for Rust applications, providing patterns
//! and tools for building fault-tolerant distributed systems.
//!
//! ## Features
//!
//! - **Circuit Breaker**: Prevent cascading failures
//! - **Retry Mechanisms**: Configurable retry strategies with backoff
//! - **Rate Limiting**: Multiple algorithms (token bucket, leaky bucket, sliding window)
//! - **Bulkhead Isolation**: Resource isolation patterns
//! - **Timeout Management**: Adaptive and hierarchical timeouts
//! - **Fallback Strategies**: Graceful degradation
//! - **Hedge Requests**: Reduce tail latency
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_resilience::prelude::*;
//! use nebula_resilience::ResilienceManager;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a resilience policy
//!     let policy = ResiliencePolicy::builder("my-service")
//!         .timeout(Duration::from_secs(30))
//!         .retry(3, Duration::from_secs(1))
//!         .circuit_breaker(5, Duration::from_secs(60))
//!         .bulkhead(10)
//!         .build();
//!
//!     // Execute with resilience
//!     let result = policy.execute(|| async {
//!         // Your potentially failing operation
//!         Ok::<_, ResilienceError>("Success")
//!     }).await?;
//!
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

// Core module with fundamental types
pub mod core;

// Pattern implementations
pub mod patterns;

// Higher-level abstractions
mod compose;
mod policy;
mod manager;

// Re-export compose, policy, and manager at root level
pub use compose::{ResilienceChain, ChainBuilder, ResilienceMiddleware};
pub use policy::{ResiliencePolicy, ResiliencePolicyBuilder, PolicyExecutor};
pub use manager::{ResilienceManager, ResilienceManagerBuilder};

// Public API - core types
pub use core::{
    ResilienceError,
    ResilienceResult,
    ErrorClass,
    ResiliencePattern,
    Executable,
};

// Public API - patterns
pub use patterns::{
    // Circuit breaker
    CircuitBreaker,
    CircuitBreakerConfig,
    CircuitState,

    // Bulkhead
    Bulkhead,
    BulkheadConfig,

    // Retry
    retry,
    retry_with_operation,
    RetryStrategy,
    RetryBuilder,

    // Timeout
    timeout,
    timeout_with_original_error,

    // Fallback
    FallbackStrategy,
    ValueFallback,

    // Rate limiting
    RateLimiter,
    RateLimiterFactory,
    TokenBucket,
    LeakyBucket,
    SlidingWindow,
    AdaptiveRateLimiter,

    // Hedge
    HedgeExecutor,
    HedgeConfig,
};

/// Prelude module for common imports
pub mod prelude {
    pub use crate::core::{ResilienceError, ResilienceResult};
    pub use crate::patterns::{
        Bulkhead,
        CircuitBreaker,
        RetryStrategy,
        timeout,
    };
    pub use crate::{
        ResiliencePolicy,
        ResilienceManager,
        ResilienceChain,
    };
}

/// Predefined policies for common scenarios
pub mod policies {
    pub use crate::policy::policies::*;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");