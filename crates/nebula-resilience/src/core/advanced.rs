//! Advanced type system features for resilience patterns
//!
//! This module leverages advanced Rust type system features:
//! - **Typestate pattern** for compile-time state machine validation
//! - **GADT-like patterns** for type-safe policy composition
//! - **Phantom variance** for proper lifetime handling
//! - **Higher-kinded type simulation** for flexible abstractions
//! - **Const evaluation** for zero-cost configuration validation
//!
//! # Typestate Pattern
//!
//! Ensures state transitions are valid at compile time:
//!
//! ```rust
//! use nebula_resilience::core::advanced::{
//!     PolicyBuilder, Unconfigured, WithRetry, WithCircuitBreaker, Complete
//! };
//!
//! // Type-safe builder - can only build after all required configurations
//! let builder = PolicyBuilder::new()
//!     .with_retry_config(3, 100)     // Transitions to WithRetry
//!     .with_circuit_breaker(5, 30_000) // Transitions to Complete
//!     .build();                      // Only available on Complete
//! ```
//!
//! # Zero-Sized Type Markers
//!
//! Zero-cost abstractions for type-level programming:
//!
//! ```rust
//! use nebula_resilience::core::advanced::{Aggressive, Conservative, Strategy, StrategyConfig};
//!
//! fn execute<S: Strategy>(config: StrategyConfig<S>) {
//!     // Different behavior based on strategy type parameter
//! }
//! ```

use std::marker::PhantomData;
use std::time::Duration;

// =============================================================================
// TYPESTATE PATTERN FOR POLICY BUILDER
// =============================================================================

/// Unconfigured state - no patterns set
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unconfigured;

/// Retry configured state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithRetry;

/// Circuit breaker configured state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithCircuitBreaker;

/// Both retry and circuit breaker configured
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Complete;

/// Type-safe policy builder using typestate pattern.
///
/// The builder transitions through states, and `build()` is only
/// available when all required configurations are set.
///
/// # State Transitions
///
/// ```text
/// Unconfigured ─┬─► WithRetry ──────────► Complete
///               │                              ▲
///               └─► WithCircuitBreaker ────────┘
/// ```
#[derive(Debug)]
pub struct PolicyBuilder<State = Unconfigured> {
    retry_attempts: Option<usize>,
    retry_base_delay_ms: Option<u64>,
    circuit_failure_threshold: Option<usize>,
    circuit_reset_timeout_ms: Option<u64>,
    timeout_ms: Option<u64>,
    _state: PhantomData<State>,
}

impl PolicyBuilder<Unconfigured> {
    /// Creates a new unconfigured policy builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            retry_attempts: None,
            retry_base_delay_ms: None,
            circuit_failure_threshold: None,
            circuit_reset_timeout_ms: None,
            timeout_ms: None,
            _state: PhantomData,
        }
    }

    /// Configure retry settings, transitioning to `WithRetry` state.
    #[must_use]
    pub const fn with_retry_config(
        self,
        attempts: usize,
        base_delay_ms: u64,
    ) -> PolicyBuilder<WithRetry> {
        PolicyBuilder {
            retry_attempts: Some(attempts),
            retry_base_delay_ms: Some(base_delay_ms),
            circuit_failure_threshold: self.circuit_failure_threshold,
            circuit_reset_timeout_ms: self.circuit_reset_timeout_ms,
            timeout_ms: self.timeout_ms,
            _state: PhantomData,
        }
    }

    /// Configure circuit breaker, transitioning to `WithCircuitBreaker` state.
    #[must_use]
    pub const fn with_circuit_breaker(
        self,
        failure_threshold: usize,
        reset_timeout_ms: u64,
    ) -> PolicyBuilder<WithCircuitBreaker> {
        PolicyBuilder {
            retry_attempts: self.retry_attempts,
            retry_base_delay_ms: self.retry_base_delay_ms,
            circuit_failure_threshold: Some(failure_threshold),
            circuit_reset_timeout_ms: Some(reset_timeout_ms),
            timeout_ms: self.timeout_ms,
            _state: PhantomData,
        }
    }
}

impl PolicyBuilder<WithRetry> {
    /// Add circuit breaker to complete the configuration.
    #[must_use]
    pub const fn with_circuit_breaker(
        self,
        failure_threshold: usize,
        reset_timeout_ms: u64,
    ) -> PolicyBuilder<Complete> {
        PolicyBuilder {
            retry_attempts: self.retry_attempts,
            retry_base_delay_ms: self.retry_base_delay_ms,
            circuit_failure_threshold: Some(failure_threshold),
            circuit_reset_timeout_ms: Some(reset_timeout_ms),
            timeout_ms: self.timeout_ms,
            _state: PhantomData,
        }
    }

    /// Set timeout (optional, doesn't change state).
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

impl PolicyBuilder<WithCircuitBreaker> {
    /// Add retry to complete the configuration.
    #[must_use]
    pub const fn with_retry_config(
        self,
        attempts: usize,
        base_delay_ms: u64,
    ) -> PolicyBuilder<Complete> {
        PolicyBuilder {
            retry_attempts: Some(attempts),
            retry_base_delay_ms: Some(base_delay_ms),
            circuit_failure_threshold: self.circuit_failure_threshold,
            circuit_reset_timeout_ms: self.circuit_reset_timeout_ms,
            timeout_ms: self.timeout_ms,
            _state: PhantomData,
        }
    }

    /// Set timeout (optional, doesn't change state).
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

impl PolicyBuilder<Complete> {
    /// Set timeout (optional).
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Build the completed policy.
    ///
    /// This method is only available when the builder is in `Complete` state,
    /// ensuring all required configurations are set at compile time.
    #[must_use]
    pub fn build(self) -> ComposedPolicy {
        ComposedPolicy {
            retry_attempts: self.retry_attempts.unwrap(),
            retry_base_delay_ms: self.retry_base_delay_ms.unwrap(),
            circuit_failure_threshold: self.circuit_failure_threshold.unwrap(),
            circuit_reset_timeout_ms: self.circuit_reset_timeout_ms.unwrap(),
            timeout_ms: self.timeout_ms,
        }
    }
}

impl Default for PolicyBuilder<Unconfigured> {
    fn default() -> Self {
        Self::new()
    }
}

/// A fully configured resilience policy.
#[derive(Debug, Clone)]
pub struct ComposedPolicy {
    /// Maximum number of retry attempts
    pub retry_attempts: usize,
    /// Base delay in milliseconds for retry backoff
    pub retry_base_delay_ms: u64,
    /// Number of failures before circuit opens
    pub circuit_failure_threshold: usize,
    /// Timeout in milliseconds before circuit attempts to reset
    pub circuit_reset_timeout_ms: u64,
    /// Optional timeout in milliseconds for operations
    pub timeout_ms: Option<u64>,
}

// =============================================================================
// ZERO-SIZED TYPE MARKERS FOR STRATEGIES
// =============================================================================

/// Marker trait for retry strategies.
pub trait Strategy: Send + Sync + 'static {
    /// Strategy name for observability.
    fn name() -> &'static str;

    /// Whether to retry immediately without delay on first failure.
    #[must_use]
    fn immediate_first_retry() -> bool {
        false
    }
}

/// Aggressive retry strategy marker.
///
/// Retries most errors with shorter delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aggressive;

impl Strategy for Aggressive {
    fn name() -> &'static str {
        "aggressive"
    }

    fn immediate_first_retry() -> bool {
        true
    }
}

/// Conservative retry strategy marker.
///
/// Only retries known transient errors with longer delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conservative;

impl Strategy for Conservative {
    fn name() -> &'static str {
        "conservative"
    }
}

/// Balanced retry strategy marker.
///
/// Middle ground between aggressive and conservative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Balanced;

impl Strategy for Balanced {
    fn name() -> &'static str {
        "balanced"
    }
}

/// Type-parameterized configuration with strategy.
#[derive(Debug, Clone)]
pub struct StrategyConfig<S: Strategy> {
    /// Maximum number of retry attempts allowed
    pub max_attempts: usize,
    /// Base delay between retry attempts
    pub base_delay: Duration,
    _strategy: PhantomData<S>,
}

impl<S: Strategy> StrategyConfig<S> {
    /// Create new strategy configuration.
    #[must_use]
    pub const fn new(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            _strategy: PhantomData,
        }
    }

    /// Get strategy name.
    #[must_use]
    pub fn strategy_name(&self) -> &'static str {
        S::name()
    }

    /// Check if immediate first retry is enabled.
    #[must_use]
    pub fn immediate_first_retry(&self) -> bool {
        S::immediate_first_retry()
    }
}

// =============================================================================
// GADT-LIKE PATTERN FOR TYPED OPERATIONS
// =============================================================================

/// Type-level representation of operation outcomes.
pub trait OperationOutcome: Send + Sync {
    /// Associated value type.
    type Value: Send;
}

/// Successful operation outcome.
#[derive(Debug)]
pub struct Success<T>(pub T);

impl<T: Send + Sync> OperationOutcome for Success<T> {
    type Value = T;
}

/// Failed operation outcome.
#[derive(Debug)]
pub struct Failure<E>(pub E);

impl<E: Send + Sync> OperationOutcome for Failure<E> {
    type Value = E;
}

/// Pending operation outcome.
#[derive(Debug)]
pub struct Pending;

impl OperationOutcome for Pending {
    type Value = ();
}

/// Type-safe operation handle with GADT-like behavior.
///
/// The type parameter ensures only valid operations on the handle.
#[derive(Debug)]
pub struct OperationHandle<O: OperationOutcome> {
    id: u64,
    _outcome: PhantomData<O>,
}

impl<O: OperationOutcome> OperationHandle<O> {
    /// Create a new handle (internal use).
    pub(crate) const fn new(id: u64) -> Self {
        Self {
            id,
            _outcome: PhantomData,
        }
    }

    /// Get operation ID.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }
}

impl OperationHandle<Pending> {
    /// Create a pending operation handle.
    #[must_use]
    pub fn pending(id: u64) -> Self {
        Self::new(id)
    }

    /// Transition to success (consumes self).
    pub fn succeed<T: Send + Sync>(self, value: T) -> (OperationHandle<Success<T>>, T) {
        (OperationHandle::new(self.id), value)
    }

    /// Transition to failure (consumes self).
    pub fn fail<E: Send + Sync>(self, error: E) -> (OperationHandle<Failure<E>>, E) {
        (OperationHandle::new(self.id), error)
    }
}

// =============================================================================
// VARIANCE MARKERS
// =============================================================================

/// Covariant marker for lifetime 'a.
///
/// Use when the type "produces" values of lifetime 'a.
#[derive(Debug, Clone, Copy)]
pub struct Covariant<'a>(PhantomData<&'a ()>);

impl Covariant<'_> {
    /// Create a new conservative strategy marker
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl Default for Covariant<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Contravariant marker for lifetime 'a.
///
/// Use when the type "consumes" values of lifetime 'a.
#[derive(Debug, Clone, Copy)]
pub struct Contravariant<'a>(PhantomData<fn(&'a ()) -> ()>);

impl Contravariant<'_> {
    /// Create a new balanced strategy marker
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl Default for Contravariant<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Invariant marker for lifetime 'a.
///
/// Use when the type both produces and consumes values of lifetime 'a.
#[derive(Debug, Clone, Copy)]
pub struct Invariant<'a>(PhantomData<fn(&'a ()) -> &'a ()>);

impl Invariant<'_> {
    /// Create a new aggressive strategy marker
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl Default for Invariant<'_> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CONST-VALIDATED CONFIGURATION
// =============================================================================

/// Trait for compile-time validated configurations.
pub trait ConstValidated {
    /// Check if configuration is valid (must be const).
    const IS_VALID: bool;

    /// Validation error message if invalid.
    const ERROR_MESSAGE: &'static str;
}

/// Compile-time validated retry configuration.
#[derive(Debug, Clone, Copy)]
pub struct ValidatedRetryConfig<
    const MAX_ATTEMPTS: usize,
    const BASE_DELAY_MS: u64,
    const MAX_DELAY_MS: u64,
> {
    _private: (),
}

impl<const MAX_ATTEMPTS: usize, const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64> ConstValidated
    for ValidatedRetryConfig<MAX_ATTEMPTS, BASE_DELAY_MS, MAX_DELAY_MS>
{
    const IS_VALID: bool = MAX_ATTEMPTS > 0
        && MAX_ATTEMPTS <= 100
        && BASE_DELAY_MS > 0
        && MAX_DELAY_MS >= BASE_DELAY_MS
        && MAX_DELAY_MS <= 300_000; // 5 minutes max

    const ERROR_MESSAGE: &'static str = if MAX_ATTEMPTS == 0 {
        "MAX_ATTEMPTS must be > 0"
    } else if MAX_ATTEMPTS > 100 {
        "MAX_ATTEMPTS must be <= 100"
    } else if BASE_DELAY_MS == 0 {
        "BASE_DELAY_MS must be > 0"
    } else if MAX_DELAY_MS < BASE_DELAY_MS {
        "MAX_DELAY_MS must be >= BASE_DELAY_MS"
    } else if MAX_DELAY_MS > 300_000 {
        "MAX_DELAY_MS must be <= 300000 (5 minutes)"
    } else {
        "Configuration is valid"
    };
}

impl<const MAX_ATTEMPTS: usize, const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64>
    ValidatedRetryConfig<MAX_ATTEMPTS, BASE_DELAY_MS, MAX_DELAY_MS>
{
    /// Create a new validated configuration.
    ///
    /// # Compile-Time Validation
    ///
    /// This function includes a compile-time check that will cause a
    /// compilation error if the configuration is invalid.
    #[must_use]
    pub const fn new() -> Self {
        // Compile-time assertion
        const { assert!(Self::IS_VALID, "Invalid retry configuration") };

        Self { _private: () }
    }

    /// Get max attempts.
    #[must_use]
    pub const fn max_attempts(&self) -> usize {
        MAX_ATTEMPTS
    }

    /// Get base delay.
    #[must_use]
    pub const fn base_delay(&self) -> Duration {
        Duration::from_millis(BASE_DELAY_MS)
    }

    /// Get max delay.
    #[must_use]
    pub const fn max_delay(&self) -> Duration {
        Duration::from_millis(MAX_DELAY_MS)
    }
}

impl<const MAX_ATTEMPTS: usize, const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64> Default
    for ValidatedRetryConfig<MAX_ATTEMPTS, BASE_DELAY_MS, MAX_DELAY_MS>
{
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_builder_typestate() {
        let policy = PolicyBuilder::new()
            .with_retry_config(3, 100)
            .with_circuit_breaker(5, 30_000)
            .with_timeout(10_000)
            .build();

        assert_eq!(policy.retry_attempts, 3);
        assert_eq!(policy.retry_base_delay_ms, 100);
        assert_eq!(policy.circuit_failure_threshold, 5);
        assert_eq!(policy.circuit_reset_timeout_ms, 30_000);
        assert_eq!(policy.timeout_ms, Some(10_000));
    }

    #[test]
    fn test_policy_builder_alternate_order() {
        // Can configure circuit breaker first, then retry
        let policy = PolicyBuilder::new()
            .with_circuit_breaker(10, 60_000)
            .with_retry_config(5, 200)
            .build();

        assert_eq!(policy.retry_attempts, 5);
        assert_eq!(policy.circuit_failure_threshold, 10);
    }

    #[test]
    fn test_strategy_markers() {
        fn check_strategy<S: Strategy>(_config: &StrategyConfig<S>) -> &'static str {
            S::name()
        }

        let aggressive = StrategyConfig::<Aggressive>::new(5, Duration::from_millis(50));
        let conservative = StrategyConfig::<Conservative>::new(3, Duration::from_millis(200));

        assert_eq!(check_strategy(&aggressive), "aggressive");
        assert_eq!(check_strategy(&conservative), "conservative");
        assert!(aggressive.immediate_first_retry());
        assert!(!conservative.immediate_first_retry());
    }

    #[test]
    fn test_operation_handle_transitions() {
        let pending = OperationHandle::<Pending>::pending(42);
        assert_eq!(pending.id(), 42);

        let (success_handle, value) = pending.succeed("ok");
        assert_eq!(success_handle.id(), 42);
        assert_eq!(value, "ok");
    }

    #[test]
    fn test_const_validated_config() {
        // Valid configuration
        const VALID: ValidatedRetryConfig<3, 100, 5000> = ValidatedRetryConfig::new();
        assert_eq!(VALID.max_attempts(), 3);
        assert_eq!(VALID.base_delay(), Duration::from_millis(100));
        assert_eq!(VALID.max_delay(), Duration::from_millis(5000));
    }

    #[test]
    fn test_variance_markers() {
        let _covariant: Covariant<'static> = Covariant::new();
        let _contravariant: Contravariant<'static> = Contravariant::new();
        let _invariant: Invariant<'static> = Invariant::new();
    }
}
