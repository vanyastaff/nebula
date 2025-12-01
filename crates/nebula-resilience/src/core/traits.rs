//! Core traits for resilience patterns with advanced type safety
//!
//! This module provides the fundamental building blocks using advanced Rust type system features:
//! - Generic Associated Types (GATs) for flexible async operations
//! - Sealed traits for controlled extensibility
//! - Type-state patterns for compile-time safety
//! - Zero-cost abstractions with phantom types

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use super::error::ResilienceError;
use super::result::ResilienceResult;

/// Sealed module to prevent external trait implementations
pub mod sealed {
    /// Sealed trait marker to prevent external implementations
    pub trait Sealed {}
}

/// Base trait for all resilience patterns with Generic Associated Types
pub trait ResiliencePattern: Send + Sync + sealed::Sealed {
    /// Pattern-specific state type
    type State: Send + Sync + fmt::Debug + Clone;

    /// Pattern-specific configuration type
    type Config: Send + Sync + fmt::Debug + Clone;

    /// Metrics type with lifetime parameter (GAT)
    type Metrics<'a>: PatternMetrics + Send + 'a
    where
        Self: 'a;

    /// Future type for async operations (GAT)
    type ExecuteFuture<'a, T>: Future<Output = ResilienceResult<T>> + Send + 'a
    where
        Self: 'a,
        T: Send + 'a;

    /// Pattern name for identification
    fn name(&self) -> &'static str;

    /// Get pattern metrics
    fn metrics(&self) -> Self::Metrics<'_>;

    /// Get current state
    fn state(&self) -> &Self::State;

    /// Reset pattern state
    fn reset(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Check if pattern is healthy
    fn is_healthy(&self) -> bool {
        self.metrics().error_rate() < 0.5
    }
}

/// Trait for executable operations with Higher-Rank Trait Bounds
pub trait Executable: ResiliencePattern {
    /// Execute an operation with this pattern using HRTB
    fn execute<'a, T, F, Fut>(&'a self, operation: F) -> Self::ExecuteFuture<'a, T>
    where
        F: for<'b> FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = ResilienceResult<T>> + Send + 'a,
        T: Send + 'a;
}

/// Type-safe retry classification using const generics
pub trait Retryable<const MAX_ATTEMPTS: usize = 3> {
    /// Error type that can be retried
    type Error: Send + Sync + fmt::Debug;

    /// Check if the error should trigger a retry
    fn is_retryable(&self) -> bool;

    /// Check if the error is terminal
    fn is_terminal(&self) -> bool;

    /// Get suggested retry delay with attempt counter
    fn retry_delay(&self, attempt: usize) -> Option<Duration> {
        if attempt >= MAX_ATTEMPTS {
            None
        } else {
            Some(Duration::from_millis(100 * (1 << attempt.min(10))))
        }
    }

    /// Get retry budget remaining
    fn retry_budget(&self, current_attempt: usize) -> usize {
        MAX_ATTEMPTS.saturating_sub(current_attempt)
    }
}

/// Pattern metrics trait with GAT support
pub trait PatternMetrics {
    /// Metric value type (could be different for different patterns)
    type Value: Send + Sync + fmt::Debug + Clone;

    /// Get a specific metric value
    fn get_metric(&self, name: &str) -> Option<Self::Value>;

    /// Calculate error rate
    fn error_rate(&self) -> f64;

    /// Calculate success rate
    fn success_rate(&self) -> f64 {
        1.0 - self.error_rate()
    }

    /// Get total operations count
    fn total_operations(&self) -> u64;
}

/// Concrete pattern metrics implementation
#[derive(Debug, Clone, Default)]
#[must_use = "PatternMetrics contains valuable information that should be used"]
pub struct StandardMetrics {
    /// Total number of calls
    pub total_calls: u64,
    /// Number of successful calls
    pub successful_calls: u64,
    /// Number of failed calls
    pub failed_calls: u64,
    /// Total latency in milliseconds
    pub total_latency_ms: u64,
    /// Minimum latency
    pub min_latency_ms: u64,
    /// Maximum latency
    pub max_latency_ms: u64,
    /// Pattern-specific metrics
    pub custom: std::collections::HashMap<String, MetricValue>,
}

/// Type-safe metric values
#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue {
    /// Counter value
    Counter(u64),
    /// Gauge value
    Gauge(f64),
    /// Histogram bucket
    Histogram {
        /// Total count of observations
        count: u64,
        /// Sum of all observed values
        sum: f64,
        /// Histogram buckets (upper bound, count)
        buckets: Vec<(f64, u64)>,
    },
    /// Duration value
    Duration(Duration),
    /// Boolean flag
    Flag(bool),
}

impl PatternMetrics for StandardMetrics {
    type Value = MetricValue;

    fn get_metric(&self, name: &str) -> Option<Self::Value> {
        self.custom.get(name).cloned()
    }

    fn error_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.failed_calls as f64 / self.total_calls as f64
        }
    }

    fn total_operations(&self) -> u64 {
        self.total_calls
    }
}

impl StandardMetrics {
    /// Calculate average latency
    #[must_use]
    pub fn avg_latency_ms(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.total_calls as f64
        }
    }

    /// Add a custom metric
    pub fn add_metric(&mut self, name: String, value: MetricValue) {
        self.custom.insert(name, value);
    }
}

/// Health check trait with GAT for flexible return types
pub trait HealthCheck: Send + Sync {
    /// Health status type
    type Status: Send + Sync + fmt::Debug;

    /// Future type for health checks
    type HealthFuture<'a>: Future<Output = Self::Status> + Send + 'a
    where
        Self: 'a;

    /// Check health status
    fn check_health(&self) -> Self::HealthFuture<'_>;
}

/// Standard health status
#[derive(Debug, Clone, PartialEq)]
#[must_use = "HealthStatus should be checked and acted upon"]
pub enum HealthStatus {
    /// Fully operational
    Healthy,
    /// Degraded performance
    Degraded {
        /// Reason for degraded status
        reason: String,
    },
    /// Not operational
    Unhealthy {
        /// Reason for unhealthy status
        reason: String,
    },
}

/// Type-safe circuit breaker states using phantom types
pub mod circuit_states {
    use std::marker::PhantomData;

    /// Closed state marker
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Closed;

    /// Open state marker
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Open;

    /// Half-open state marker
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HalfOpen;

    /// Type-safe circuit state with phantom type for compile-time state tracking
    ///
    /// This is distinct from the runtime `CircuitState` enum in `patterns::circuit_breaker`.
    /// Use this for typestate pattern implementations where state transitions are
    /// enforced at compile time.
    #[derive(Debug, Clone)]
    pub struct TypestateCircuitState<S> {
        _marker: PhantomData<S>,
        /// Metadata about the state
        pub metadata: StateMetadata,
    }

    /// Metadata associated with circuit state
    #[derive(Debug, Clone)]
    pub struct StateMetadata {
        /// When the state was entered
        pub entered_at: std::time::Instant,
        /// Failure count in current state
        pub failure_count: usize,
        /// Success count in current state
        pub success_count: usize,
    }

    impl<S> TypestateCircuitState<S> {
        /// Create a new state with metadata
        pub fn new(failure_count: usize, success_count: usize) -> Self {
            Self {
                _marker: PhantomData,
                metadata: StateMetadata {
                    entered_at: std::time::Instant::now(),
                    failure_count,
                    success_count,
                },
            }
        }

        /// Get state type name
        pub fn type_name(&self) -> &'static str {
            std::any::type_name::<S>()
                .split("::")
                .last()
                .unwrap_or("Unknown")
        }
    }

    /// State transition trait for type-safe transitions
    pub trait StateTransition<From, To> {
        /// Transition from one state to another
        fn transition(from: TypestateCircuitState<From>) -> TypestateCircuitState<To>;
    }

    /// Closed -> Open transition
    impl StateTransition<Closed, Open> for TypestateCircuitState<Open> {
        fn transition(from: TypestateCircuitState<Closed>) -> TypestateCircuitState<Open> {
            TypestateCircuitState::new(from.metadata.failure_count, 0)
        }
    }

    /// Open -> HalfOpen transition
    impl StateTransition<Open, HalfOpen> for TypestateCircuitState<HalfOpen> {
        fn transition(from: TypestateCircuitState<Open>) -> TypestateCircuitState<HalfOpen> {
            TypestateCircuitState::new(from.metadata.failure_count, 0)
        }
    }

    /// HalfOpen -> Closed transition
    impl StateTransition<HalfOpen, Closed> for TypestateCircuitState<Closed> {
        fn transition(from: TypestateCircuitState<HalfOpen>) -> TypestateCircuitState<Closed> {
            TypestateCircuitState::new(0, from.metadata.success_count)
        }
    }

    /// HalfOpen -> Open transition
    impl StateTransition<HalfOpen, Open> for TypestateCircuitState<Open> {
        fn transition(from: TypestateCircuitState<HalfOpen>) -> TypestateCircuitState<Open> {
            TypestateCircuitState::new(from.metadata.failure_count + 1, 0)
        }
    }
}

/// Re-export circuit state types
pub use circuit_states::*;

/// Type-safe configuration trait with const generics
pub trait Config<const VALIDATION_LEVEL: u8 = 1>: Send + Sync + fmt::Debug + Clone {
    /// Validation error type
    type ValidationError: Send + Sync + fmt::Debug + std::error::Error;

    /// Validate configuration at compile time where possible
    fn is_valid_const(&self) -> bool {
        true // Override in implementations
    }

    /// Runtime validation for complex rules
    fn validate(&self) -> Result<(), Self::ValidationError>;

    /// Merge with another configuration
    fn merge(&mut self, other: &Self);
}

/// Zero-cost wrapper for validated configurations
#[derive(Debug, Clone)]
pub struct Validated<T> {
    inner: T,
    _validated: PhantomData<fn() -> ()>, // Prevents external construction
}

impl<T> Validated<T>
where
    T: Config,
{
    /// Create a validated configuration (only callable after validation)
    pub(crate) fn new_validated(config: T) -> Self {
        Self {
            inner: config,
            _validated: PhantomData,
        }
    }

    /// Get the inner configuration (guaranteed to be valid)
    pub fn get(&self) -> &T {
        &self.inner
    }

    /// Consume and return the inner configuration
    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// Extension trait for configuration validation
pub trait ConfigExt<T: Config + Clone> {
    /// Validate and wrap in Validated type
    fn validated(self) -> Result<Validated<T>, T::ValidationError>;
}

impl<T: Config + Clone> ConfigExt<T> for T {
    fn validated(self) -> Result<Validated<T>, T::ValidationError> {
        Config::validate(&self)?;
        Ok(Validated::new_validated(self))
    }
}

/// Implement sealed trait for standard types
impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> sealed::Sealed
    for super::super::patterns::circuit_breaker::CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
}
impl sealed::Sealed for super::super::patterns::bulkhead::Bulkhead {}

/// Default implementation of Retryable for ResilienceError
impl<const MAX_ATTEMPTS: usize> Retryable<MAX_ATTEMPTS> for ResilienceError {
    type Error = ResilienceError;

    fn is_retryable(&self) -> bool {
        self.is_retryable()
    }

    fn is_terminal(&self) -> bool {
        self.is_terminal()
    }

    fn retry_delay(&self, attempt: usize) -> Option<Duration> {
        if let Some(retry_after) = self.retry_after() {
            Some(retry_after)
        } else if attempt >= MAX_ATTEMPTS {
            None
        } else {
            // Exponential backoff with jitter
            let base_delay = Duration::from_millis(100);
            let exp_delay = base_delay * (1 << attempt.min(10));
            let jitter = Duration::from_millis(fastrand::u64(0..=exp_delay.as_millis() as u64));
            Some(exp_delay + jitter)
        }
    }
}

/// Type-safe timeout configuration with const generics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutConfig<const DEFAULT_MS: u64 = 30000> {
    timeout_ms: u64,
}

impl<const DEFAULT_MS: u64> Default for TimeoutConfig<DEFAULT_MS> {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_MS,
        }
    }
}

impl<const DEFAULT_MS: u64> TimeoutConfig<DEFAULT_MS> {
    /// Create new timeout configuration
    pub fn new(timeout_ms: u64) -> Self {
        Self { timeout_ms }
    }

    /// Get timeout as Duration
    pub fn as_duration(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    /// Check if timeout is reasonable
    pub fn is_reasonable(&self) -> bool {
        self.timeout_ms > 0 && self.timeout_ms < 300_000 // Max 5 minutes
    }
}

impl<const DEFAULT_MS: u64> Config for TimeoutConfig<DEFAULT_MS> {
    type ValidationError = TimeoutValidationError;

    fn is_valid_const(&self) -> bool {
        self.is_reasonable()
    }

    fn validate(&self) -> Result<(), Self::ValidationError> {
        if self.is_reasonable() {
            Ok(())
        } else {
            Err(TimeoutValidationError {
                message: "Timeout must be between 1ms and 5 minutes",
            })
        }
    }

    fn merge(&mut self, other: &Self) {
        // Take the minimum timeout for safety
        self.timeout_ms = self.timeout_ms.min(other.timeout_ms);
    }
}

/// Timeout validation error
#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutValidationError {
    /// Error message describing the validation failure
    pub message: &'static str,
}

impl fmt::Display for TimeoutValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TimeoutValidationError {}

/// Function for creating validated timeouts
pub fn timeout<const MS: u64>() -> TimeoutConfig<MS> {
    let config = TimeoutConfig::new(MS);
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_validation() {
        let config = TimeoutConfig::<5000>::new(1000);
        assert!(config.validate().is_ok());

        let invalid_config = TimeoutConfig::<5000>::new(0);
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_const_timeout_creation() {
        let _valid_timeout: TimeoutConfig<5000> = timeout::<5000>();
        assert!(_valid_timeout.is_reasonable());
    }

    #[test]
    fn test_metric_value_operations() {
        let mut metrics = StandardMetrics::default();
        metrics.add_metric("test_counter".to_string(), MetricValue::Counter(42));

        if let Some(MetricValue::Counter(value)) = metrics.get_metric("test_counter") {
            assert_eq!(value, 42);
        } else {
            panic!("Expected counter metric");
        }
    }

    #[test]
    fn test_circuit_state_transitions() {
        let closed_state = TypestateCircuitState::<Closed>::new(0, 10);
        let open_state = TypestateCircuitState::<Open>::transition(closed_state);

        assert_eq!(open_state.type_name(), "Open");
        assert_eq!(open_state.metadata.success_count, 0);
    }
}
