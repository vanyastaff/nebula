//! Resilience patterns for building fault-tolerant systems with advanced type safety
//!
//! This crate provides resilience patterns including retry, circuit breaker,
//! bulkhead, rate limiter, and timeout functionality using advanced Rust type system features:
//!
//! - **Const generics** for compile-time configuration validation
//! - **Phantom types** for zero-cost state safety
//! - **GATs (Generic Associated Types)** for flexible async operations
//! - **Sealed traits** for controlled extensibility
//! - **Type-state patterns** for compile-time correctness
//! - **Zero-cost abstractions** with marker types
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use nebula_resilience::prelude::*;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Type-safe circuit breaker with compile-time configuration
//!     let config = CircuitBreakerConfig::<5, 30_000>::new()
//!         .with_half_open_limit(3)
//!         .with_min_operations(10);
//!
//!     let breaker = CircuitBreaker::new(config)?;
//!
//!     // Type-safe retry with const generics
//!     let retry_strategy = exponential_retry::<3>()?;
//!
//!     // Execute with both patterns
//!     let result = breaker.execute(|| async {
//!         retry_strategy.execute_resilient(|| async {
//!             // Your operation here
//!             Ok::<_, ResilienceError>("success")
//!         }).await
//!     }).await;
//!     Ok(())
//! }
//! ```
//!
//! # Type-Safe Circuit Breaker
//!
//! ```rust,no_run
//! use nebula_resilience::{CircuitBreaker, CircuitBreakerConfig, ResilienceError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Compile-time validated configuration with const generics
//!     let config = CircuitBreakerConfig::<3, 10_000>::new()
//!         .with_half_open_limit(2)
//!         .with_min_operations(5);
//!
//!     let breaker = CircuitBreaker::new(config)?;
//!
//!     let result = breaker.execute(|| async {
//!         Ok::<_, ResilienceError>("success")
//!     }).await;
//!     Ok(())
//! }
//! ```
//!
//! # Advanced Retry Strategies
//!
//! ```rust,no_run
//! use nebula_resilience::{
//!     RetryStrategy, ExponentialBackoff, ConservativeCondition,
//!     RetryConfig, JitterPolicy, ResilienceError
//! };
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Type-safe retry with const generics and zero-cost abstractions
//!     let config = RetryConfig::new(
//!         ExponentialBackoff::<100, 20, 5000>::default(),
//!         ConservativeCondition::<3>::new()
//!     ).with_jitter(JitterPolicy::Equal);
//!
//!     let strategy = RetryStrategy::new(config)?;
//!
//!     let (result, stats) = strategy.execute(|| async {
//!         Ok::<_, ResilienceError>("success")
//!     }).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Typestate Pattern Example
//!
//! The circuit breaker uses typestate pattern for compile-time state safety:
//!
//! ```rust
//! use nebula_resilience::patterns::circuit_breaker::{
//!     CircuitBreaker, CircuitBreakerConfig, Closed, Open, HalfOpen
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Type-safe builder that tracks configuration state
//! let config = CircuitBreakerConfig::<5, 30_000>::new()
//!     .with_half_open_limit(3);
//!
//! let breaker = CircuitBreaker::new(config)?;
//! // Circuit breaker starts in Closed state (type: CircuitBreaker<5, 30_000>)
//! # Ok(())
//! # }
//! ```
//!
//! # Const Generic Validation
//!
//! Configuration is validated at compile time using const generics:
//!
//! ```rust
//! use nebula_resilience::patterns::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Compile-time validated circuit breaker
//! // FAILURE_THRESHOLD=5, RESET_TIMEOUT_MS=30000
//! let breaker = CircuitBreaker::<5, 30_000>::new(
//!     CircuitBreakerConfig::new()
//! )?;
//!
//! // The const generics ensure configuration validity at compile time
//! // This prevents runtime configuration errors
//! # Ok(())
//! # }
//! ```
//!
//! # Advanced Type Features
//!
//! This crate demonstrates several advanced Rust type system features:
//!
//! - **Const Generics**: Compile-time configuration validation
//! - **Phantom Types**: Zero-cost state tracking without runtime overhead
//! - **GATs**: Flexible async operation handling
//! - **Sealed Traits**: Controlled API extensibility
//! - **Typestate Pattern**: Compile-time state machine correctness

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::self_only_used_in_recursion)]
#![allow(clippy::double_must_use)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::unused_self)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::let_unit_value)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::trivially_copy_pass_by_ref)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::new_without_default)]
#![allow(clippy::unused_async)]
#![allow(dead_code)]
#![warn(missing_docs)]
#![deny(unsafe_code)]

// Core modules with advanced type system features
pub mod core;
pub mod helpers;
mod manager;
pub mod observability;
pub mod patterns;
mod policy;
pub mod retryable;

// High-level composition and management
mod compose;

// Re-exports from core with type safety
pub use core::{
    // Error handling and results
    ResilienceError,
    ResilienceResult,

    // Advanced type system features
    advanced::{
        Aggressive,
        Balanced,
        Complete,
        ComposedPolicy,
        Conservative,
        // Const-validated configurations
        ConstValidated,
        Contravariant,
        // Variance markers
        Covariant,
        Failure,
        Invariant,
        OperationHandle,
        // GADT-like operation handles
        OperationOutcome,
        Pending,
        // Typestate pattern for builders
        PolicyBuilder as TypestatePolicyBuilder,
        // Strategy markers (ZST)
        Strategy,
        StrategyConfig,
        Success,
        Unconfigured,
        ValidatedRetryConfig,
        WithCircuitBreaker,
        WithRetry,
    },

    // Category traits
    category::{FallbackPattern, FlowControlPattern, ProtectionPattern, RateLimitingPattern},
    // Configuration types with const generics
    config::{ConfigError, ConfigResult, ResilienceConfig},

    // Advanced trait system
    traits::{
        // Configuration validation
        Config,
        ConfigExt,
        Executable,
        MetricValue,
        PatternMetrics,
        // Type-safe traits with GATs
        ResiliencePattern,
        // Retry traits with const generics
        Retryable as CoreRetryable,
        StandardMetrics,
        // Zero-cost timeout configuration
        TimeoutConfig,
        Validated,
        // Type-state circuit breaker states (compile-time state tracking)
        circuit_states::{Closed, HalfOpen, Open, StateTransition, TypestateCircuitState},
        timeout,
    },

    // Type-safe newtypes
    types::{
        DurationExt, FailureThreshold, MaxConcurrency, RateLimit, ResilienceResultExt, RetryCount,
        Timeout as TimeoutNewtype,
    },
};

// Re-exports from patterns with advanced features
pub use patterns::{
    // Other patterns (maintained for compatibility)
    bulkhead::{Bulkhead, BulkheadConfig},
    // Type-safe circuit breaker
    circuit_breaker::{
        CircuitBreaker,
        CircuitBreakerConfig,
        CircuitBreakerStats,
        FastCircuitBreaker,
        SlowCircuitBreaker,
        StandardCircuitBreaker,
        // Circuit state enum for runtime state checking
        State as CircuitState,
        fast_config,
        slow_config,
        standard_config,
    },

    fallback::{AnyStringFallbackStrategy, FallbackStrategy, ValueFallback},
    hedge::{HedgeConfig, HedgeExecutor},
    rate_limiter::{
        AdaptiveRateLimiter, AnyRateLimiter, LeakyBucket, RateLimiter, SlidingWindow, TokenBucket,
    },
    // Advanced retry strategies
    retry::{
        AggressiveCondition,
        AggressiveRetry,
        // Backoff policies with compile-time validation
        BackoffPolicy,
        ConservativeCondition,
        CustomBackoff,

        ExponentialBackoff,
        FixedDelay,
        // Jitter policies
        JitterPolicy,

        LinearBackoff,
        QuickRetry,
        // Retry conditions with type safety
        RetryCondition,
        RetryConfig,
        RetryStats,

        // Type-safe retry strategy with const generics
        RetryStrategy,
        // Convenience aliases
        StandardRetry,
        TimeBasedCondition,

        TimeConstrainedRetry,

        aggressive_retry,
        // Helper functions
        exponential_retry,
        fixed_retry,
        retry,
        retry_with_backoff,
    },

    timeout::{timeout as timeout_fn, timeout_with_original_error},
};

// High-level abstractions
pub use compose::{LayerBuilder, ResilienceChain, ResilienceLayer};
pub use manager::{
    PolicyBuilder, ResilienceManager, RetryableOperation, UnTypedServiceMetrics as ServiceMetrics,
};
pub use policy::{PolicyMetadata, ResiliencePolicy};

// Re-export Retryable trait for backward compatibility (already exported in core traits)

/// Prelude module with the most commonly used types and traits
///
/// This module provides a convenient way to import the most frequently used
/// types and traits from the nebula-resilience crate with advanced type safety features.
///
/// # Example
///
/// ```rust
/// use nebula_resilience::prelude::*;
///
/// // Create standard circuit breaker and retry strategy
/// let breaker = StandardCircuitBreaker::default();
/// let retry = exponential_retry::<3>().unwrap();
/// ```
pub mod prelude {
    // Core error and result types
    pub use crate::core::{ResilienceError, ResilienceResult};

    // Configuration with type safety
    pub use crate::core::{
        ConfigError, ConfigResult, ResilienceConfig,
        traits::{Config, ConfigExt, TimeoutConfig, Validated},
    };

    // Advanced trait system
    pub use crate::core::traits::{
        Executable, HealthCheck, HealthStatus, PatternMetrics, ResiliencePattern, Retryable,
    };

    // Type-safe newtypes (from rust_advanced_types.md patterns)
    pub use crate::core::types::{
        DurationExt, FailureThreshold, MaxConcurrency, RateLimit, ResilienceResultExt, RetryCount,
        Timeout,
    };

    // Advanced type system features
    pub use crate::core::advanced::{
        Aggressive,
        Balanced,
        Complete,
        ComposedPolicy,
        Conservative,
        // Const-validated configs
        ConstValidated,
        // Typestate pattern (use TypestatePolicyBuilder for compile-time validation)
        PolicyBuilder as TypestatePolicyBuilder,
        // Strategy markers (ZST)
        Strategy,
        StrategyConfig,
        Unconfigured,
        ValidatedRetryConfig,
        WithCircuitBreaker,
        WithRetry,
    };

    // Runtime policy builder (use PolicyBuilder for runtime configuration)
    pub use crate::manager::PolicyBuilder;

    // Category traits (Sealed pattern)
    pub use crate::core::category::{
        FallbackPattern, FlowControlPattern, ProtectionPattern, RateLimitingPattern,
    };

    // Type-safe circuit breaker
    pub use crate::patterns::circuit_breaker::{
        CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats, FastCircuitBreaker,
        SlowCircuitBreaker, StandardCircuitBreaker, fast_config, slow_config, standard_config,
    };

    // Advanced retry strategies
    pub use crate::patterns::retry::{
        AggressiveCondition, AggressiveRetry, BackoffPolicy, ConservativeCondition,
        ExponentialBackoff, FixedDelay, JitterPolicy, LinearBackoff, QuickRetry, RetryCondition,
        RetryConfig, RetryStats, RetryStrategy, StandardRetry, aggressive_retry, exponential_retry,
        fixed_retry, retry,
    };

    // Other essential patterns
    pub use crate::patterns::{
        bulkhead::{Bulkhead, BulkheadConfig},
        timeout::timeout as timeout_fn,
    };

    // High-level abstractions
    pub use crate::{ResilienceChain, ResilienceManager, ResiliencePolicy};

    // Re-export nebula ecosystem for convenience
    pub use nebula_config::ConfigSource;
    pub use nebula_log::{debug, error, info, warn};
    pub use nebula_value::Value;

    // Standard library re-exports for convenience
    pub use std::time::Duration;
}

/// Advanced configuration builder with type safety
///
/// This module provides builder patterns that leverage the Rust type system
/// to ensure configuration validity at compile time.
///
/// # Example
///
/// ```rust
/// use nebula_resilience::builder::*;
///
/// // Create a resilience builder with compile-time configuration
/// let builder = ResilienceBuilder::new()
///     .with_circuit_breaker::<5, 30_000>(|config| {
///         config.with_half_open_limit(3)
///               .with_min_operations(10)
///     });
/// ```
pub mod builder {
    use crate::prelude::{
        AggressiveCondition, BackoffPolicy, CircuitBreakerConfig, ConservativeCondition,
        ExponentialBackoff, FixedDelay, RetryConfig, StandardRetry,
    };
    use std::marker::PhantomData;

    /// Type-safe resilience builder
    pub struct ResilienceBuilder<CB = (), R = ()> {
        circuit_breaker: CB,
        retry: R,
        _marker: PhantomData<(CB, R)>,
    }

    impl ResilienceBuilder<(), ()> {
        /// Create a new resilience builder
        #[must_use]
        pub const fn new() -> Self {
            Self {
                circuit_breaker: (),
                retry: (),
                _marker: PhantomData,
            }
        }
    }

    impl<CB, R> ResilienceBuilder<CB, R> {
        /// Add circuit breaker with compile-time configuration
        pub fn with_circuit_breaker<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>(
            self,
            config_fn: impl FnOnce(
                CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
            )
                -> CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
        ) -> ResilienceBuilder<CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>, R>
        {
            let config = config_fn(CircuitBreakerConfig::new());
            ResilienceBuilder {
                circuit_breaker: config,
                retry: self.retry,
                _marker: PhantomData,
            }
        }

        /// Add retry strategy with compile-time configuration
        pub fn with_retry<const MAX_ATTEMPTS: usize>(
            self,
            config_fn: impl FnOnce(RetryBuilder<MAX_ATTEMPTS>) -> StandardRetry,
        ) -> ResilienceBuilder<CB, StandardRetry> {
            let config = config_fn(RetryBuilder::new());
            ResilienceBuilder {
                circuit_breaker: self.circuit_breaker,
                retry: config,
                _marker: PhantomData,
            }
        }
    }

    /// Retry configuration builder with type safety
    pub struct RetryBuilder<const MAX_ATTEMPTS: usize> {
        _marker: PhantomData<()>,
    }

    impl<const MAX_ATTEMPTS: usize> RetryBuilder<MAX_ATTEMPTS> {
        /// Create new retry builder
        #[must_use]
        pub const fn new() -> Self {
            Self {
                _marker: PhantomData,
            }
        }

        /// Configure exponential backoff
        #[must_use]
        pub fn exponential_backoff<const BASE_DELAY_MS: u64, const MULTIPLIER_X10: u64>(
            self,
        ) -> PartialRetryConfig<ExponentialBackoff<BASE_DELAY_MS, MULTIPLIER_X10>, MAX_ATTEMPTS>
        {
            PartialRetryConfig {
                backoff: ExponentialBackoff::default(),
                _marker: PhantomData,
            }
        }

        /// Configure fixed delay
        #[must_use]
        pub fn fixed_delay<const DELAY_MS: u64>(
            self,
        ) -> PartialRetryConfig<FixedDelay<DELAY_MS>, MAX_ATTEMPTS> {
            PartialRetryConfig {
                backoff: FixedDelay::default(),
                _marker: PhantomData,
            }
        }
    }

    /// Partial retry configuration for method chaining
    pub struct PartialRetryConfig<B: BackoffPolicy, const MAX_ATTEMPTS: usize> {
        backoff: B,
        _marker: PhantomData<()>,
    }

    impl<B: BackoffPolicy, const MAX_ATTEMPTS: usize> PartialRetryConfig<B, MAX_ATTEMPTS> {
        /// Use conservative retry condition
        pub fn conservative_condition(self) -> RetryConfig<B, ConservativeCondition<MAX_ATTEMPTS>> {
            RetryConfig::new(self.backoff, ConservativeCondition::new())
        }

        /// Use aggressive retry condition
        pub fn aggressive_condition(self) -> RetryConfig<B, AggressiveCondition<MAX_ATTEMPTS>> {
            RetryConfig::new(self.backoff, AggressiveCondition::new())
        }

        /// Use time-based retry condition
        pub fn time_based_condition<const MAX_DURATION_MS: u64>(
            self,
        ) -> RetryConfig<B, crate::patterns::retry::TimeBasedCondition<MAX_DURATION_MS>> {
            RetryConfig::new(
                self.backoff,
                crate::patterns::retry::TimeBasedCondition::new(MAX_ATTEMPTS),
            )
        }
    }

    // Note: with_jitter and with_max_duration are already defined on RetryConfig in retry.rs
}

/// Type-level constants for common configurations.
pub mod constants {
    use std::time::Duration;

    /// Default timeout duration (30 seconds).
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// Default number of retry attempts.
    pub const DEFAULT_RETRY_ATTEMPTS: usize = 3;
    /// Default circuit breaker failure threshold.
    pub const DEFAULT_FAILURE_THRESHOLD: usize = 5;
    /// Default rate limit (requests per second).
    pub const DEFAULT_RATE_LIMIT: f64 = 100.0;
}

/// Utility functions for type-safe resilience patterns
pub mod utils {
    use crate::prelude::{
        AggressiveRetry, ConfigResult, FastCircuitBreaker, QuickRetry, SlowCircuitBreaker,
        StandardCircuitBreaker, StandardRetry, aggressive_retry, exponential_retry, fixed_retry,
    };

    /// Create a standard resilience setup for HTTP clients
    pub fn http_resilience() -> ConfigResult<(StandardCircuitBreaker, StandardRetry)> {
        let breaker = StandardCircuitBreaker::default();
        let retry = exponential_retry::<3>()?;
        Ok((breaker, retry))
    }

    /// Create a fast-fail setup for real-time operations
    pub fn realtime_resilience() -> ConfigResult<(FastCircuitBreaker, QuickRetry)> {
        let breaker = FastCircuitBreaker::default();
        let retry = fixed_retry::<50, 2>()?;
        Ok((breaker, retry))
    }

    /// Create a resilient setup for batch operations
    pub fn batch_resilience() -> ConfigResult<(SlowCircuitBreaker, AggressiveRetry)> {
        let breaker = SlowCircuitBreaker::default();
        let retry = aggressive_retry::<5>()?;
        Ok((breaker, retry))
    }
}

/// Library version with compile-time embedding
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_configurations() {
        // Test that configurations can be created
        let _fast_cb = fast_config();
        let _standard_cb = standard_config();
        let _slow_cb = slow_config();
    }

    #[tokio::test]
    async fn test_prelude_imports() {
        use crate::prelude::*;

        // Should be able to create standard configurations
        let _breaker = StandardCircuitBreaker::default();
        let _retry = exponential_retry::<3>().expect("Should create retry strategy");
    }

    #[tokio::test]
    async fn test_utility_functions() {
        let (breaker, retry) = utils::http_resilience().expect("Should create HTTP resilience");

        // Test that they work together
        let result = breaker
            .execute(|| async {
                retry
                    .execute_resilient(|| async { Ok::<_, ResilienceError>("success") })
                    .await
            })
            .await;

        assert!(result.is_ok());
    }

    #[test]
    fn test_builder_pattern() {
        use crate::builder::*;

        let _builder = ResilienceBuilder::new()
            .with_circuit_breaker::<5, 30_000>(|config| config.with_half_open_limit(3));
        // .with_retry::<3>(|retry| {
        //     retry.exponential_backoff::<100, 20>()
        //          .conservative_condition()
        // });
    }
}
