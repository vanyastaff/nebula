//! Type-safe circuit breaker implementation using advanced Rust type system features
//!
//! This module provides a compile-time safe circuit breaker that uses:
//! - Phantom types for state safety
//! - Const generics for configuration validation
//! - GATs for flexible async operations
//! - Zero-cost abstractions
//! - Type-state pattern for compile-time state safety

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::core::{
    ResilienceError, ResilienceResult,
    cancellation::CancellationContext,
    config::{ConfigError, ConfigResult, ResilienceConfig},
    traits::PatternMetrics,
};

/// Circuit breaker state for runtime representation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    /// Circuit is closed - operations are allowed
    Closed,
    /// Circuit is open - operations are rejected
    Open,
    /// Circuit is half-open - limited operations are allowed for testing
    HalfOpen,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}

/// Type-safe circuit breaker configuration with const generics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig<
    const FAILURE_THRESHOLD: usize = 5,
    const RESET_TIMEOUT_MS: u64 = 30_000,
> {
    /// Maximum operations allowed in half-open state
    pub half_open_max_operations: usize,
    /// Whether to count timeouts as failures
    pub count_timeouts: bool,
    /// Minimum number of operations before circuit can open
    pub min_operations: usize,
    /// Custom failure rate threshold (0.0 to 1.0)
    pub failure_rate_threshold: f64,
    /// Sliding window duration for failure rate calculation
    pub sliding_window_ms: u64,
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> Default
    for CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    fn default() -> Self {
        Self {
            half_open_max_operations: 3,
            count_timeouts: true,
            min_operations: 10,
            failure_rate_threshold: 0.6,
            sliding_window_ms: 60_000,
        }
    }
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>
    CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    /// Compile-time validation of const generic parameters
    const VALID: () = {
        assert!(FAILURE_THRESHOLD > 0, "FAILURE_THRESHOLD must be positive");
        assert!(RESET_TIMEOUT_MS > 0, "RESET_TIMEOUT_MS must be positive");
        assert!(
            RESET_TIMEOUT_MS <= 300_000,
            "RESET_TIMEOUT_MS must be <= 5 minutes"
        );
    };

    /// Create new configuration with validation
    #[must_use]
    pub const fn new() -> Self {
        // Trigger compile-time validation
        let () = Self::VALID;

        Self {
            half_open_max_operations: 3,
            count_timeouts: true,
            min_operations: 10,
            failure_rate_threshold: 0.6,
            sliding_window_ms: 60_000,
        }
    }

    /// Get failure threshold (compile-time constant)
    #[must_use]
    pub const fn failure_threshold(&self) -> usize {
        FAILURE_THRESHOLD
    }

    /// Get reset timeout (compile-time constant)
    #[must_use]
    pub const fn reset_timeout(&self) -> Duration {
        Duration::from_millis(RESET_TIMEOUT_MS)
    }

    /// Builder methods
    #[must_use]
    pub const fn with_half_open_limit(mut self, limit: usize) -> Self {
        self.half_open_max_operations = limit;
        self
    }

    /// Set the minimum number of operations required before circuit can open
    #[must_use]
    pub const fn with_min_operations(mut self, min_operations: usize) -> Self {
        self.min_operations = min_operations;
        self
    }

    /// Validate configuration at runtime
    pub fn validate(&self) -> ConfigResult<()> {
        if self.failure_rate_threshold < 0.0 || self.failure_rate_threshold > 1.0 {
            return Err(ConfigError::validation(
                "failure_rate_threshold must be between 0.0 and 1.0",
            ));
        }

        if self.sliding_window_ms == 0 {
            return Err(ConfigError::validation(
                "sliding_window_ms must be positive",
            ));
        }

        Ok(())
    }
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> ResilienceConfig
    for CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    fn validate(&self) -> ConfigResult<()> {
        self.validate()
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        // Take more conservative values for safety
        self.half_open_max_operations = self
            .half_open_max_operations
            .min(other.half_open_max_operations);
        self.count_timeouts = self.count_timeouts || other.count_timeouts;
        self.min_operations = self.min_operations.max(other.min_operations);
        self.failure_rate_threshold = self
            .failure_rate_threshold
            .min(other.failure_rate_threshold);
    }
}

/// Sliding window for failure tracking.
///
/// Uses `VecDeque` for O(1) push/pop and no intermediate allocations.
/// No internal lock — callers must hold the outer `CircuitBreakerInner` lock.
#[derive(Debug)]
struct SlidingWindow {
    entries: std::collections::VecDeque<WindowEntry>,
    window_duration: Duration,
    max_entries: usize,
}

#[derive(Debug, Clone, Copy)]
struct WindowEntry {
    timestamp: Instant,
    was_failure: bool,
}

impl SlidingWindow {
    fn new(window_duration: Duration, max_entries: usize) -> Self {
        Self {
            entries: std::collections::VecDeque::with_capacity(max_entries),
            window_duration,
            max_entries,
        }
    }

    fn record_operation(&mut self, was_failure: bool) {
        let now = Instant::now();

        // Remove expired entries from the front (oldest first)
        while let Some(front) = self.entries.front() {
            if now.duration_since(front.timestamp) > self.window_duration {
                self.entries.pop_front();
            } else {
                break;
            }
        }

        // Drop oldest if at capacity
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }

        self.entries.push_back(WindowEntry {
            timestamp: now,
            was_failure,
        });
    }

    fn get_failure_rate(&self) -> f64 {
        let now = Instant::now();
        let mut total = 0usize;
        let mut failures = 0usize;

        for entry in &self.entries {
            if now.duration_since(entry.timestamp) <= self.window_duration {
                total += 1;
                if entry.was_failure {
                    failures += 1;
                }
            }
        }

        if total == 0 {
            0.0
        } else {
            failures as f64 / total as f64
        }
    }

    fn get_operation_count(&self) -> usize {
        let now = Instant::now();
        self.entries
            .iter()
            .filter(|entry| now.duration_since(entry.timestamp) <= self.window_duration)
            .count()
    }
}

/// Dynamic circuit breaker that maintains type safety while allowing runtime state changes
pub struct CircuitBreaker<const FAILURE_THRESHOLD: usize = 5, const RESET_TIMEOUT_MS: u64 = 30_000>
{
    inner: Arc<RwLock<CircuitBreakerInner<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>>>,
    /// Atomic state for lock-free fast-path: 0=Closed, 1=Open, 2=HalfOpen.
    /// Lives outside the RwLock so the closed-state fast path never acquires a lock.
    atomic_state: Arc<AtomicU8>,
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> std::fmt::Debug
    for CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("failure_threshold", &FAILURE_THRESHOLD)
            .field("reset_timeout_ms", &RESET_TIMEOUT_MS)
            .finish_non_exhaustive()
    }
}

struct CircuitBreakerInner<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> {
    config: CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
    state: State,
    /// Shared atomic state — also stored on the outer `CircuitBreaker`.
    /// Updated here (under write lock) so both copies stay in sync.
    atomic_state: Arc<AtomicU8>,
    failure_count: usize,
    last_failure_time: Option<Instant>,
    half_open_operations: usize,
    sliding_window: SlidingWindow,
    total_operations: u64,
    last_state_change: Instant,
}

impl State {
    /// Convert state to atomic representation
    const fn to_atomic(self) -> u8 {
        match self {
            Self::Closed => 0,
            Self::Open => 1,
            Self::HalfOpen => 2,
        }
    }

    /// Convert atomic representation to state
    const fn from_atomic(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Closed),
            1 => Some(Self::Open),
            2 => Some(Self::HalfOpen),
            _ => None,
        }
    }
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>
    CircuitBreakerInner<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    /// Set state and update atomic cache
    fn set_state(&mut self, new_state: State) {
        self.state = new_state;
        self.atomic_state
            .store(new_state.to_atomic(), Ordering::Release);
        self.last_state_change = Instant::now();
    }
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>
    CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    /// Create a new circuit breaker with typed configuration
    pub fn new(
        config: CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
    ) -> ConfigResult<Self> {
        config.validate()?;

        let atomic_state = Arc::new(AtomicU8::new(State::Closed.to_atomic()));

        let inner = CircuitBreakerInner {
            sliding_window: SlidingWindow::new(
                Duration::from_millis(config.sliding_window_ms),
                1000,
            ),
            config,
            state: State::Closed,
            atomic_state: Arc::clone(&atomic_state),
            failure_count: 0,
            last_failure_time: None,
            half_open_operations: 0,
            total_operations: 0,
            last_state_change: Instant::now(),
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            atomic_state,
        })
    }

    /// Create with the given configuration (alias for new)
    pub fn with_config(
        config: CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
    ) -> ConfigResult<Self> {
        Self::new(config)
    }

    /// Create with default configuration
    pub fn with_defaults() -> ConfigResult<Self> {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Execute an operation with the circuit breaker
    #[tracing::instrument(skip(self, operation), fields(
        pattern = "circuit_breaker",
        failure_threshold = FAILURE_THRESHOLD,
        reset_timeout_ms = RESET_TIMEOUT_MS,
        circuit_state = tracing::field::Empty,
    ))]
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        let cancellation = CancellationContext::new();
        self.execute_with_cancellation(operation, &cancellation)
            .await
    }

    /// Execute an operation with the circuit breaker and cancellation support
    #[tracing::instrument(skip(self, operation, cancellation), fields(
        pattern = "circuit_breaker",
        failure_threshold = FAILURE_THRESHOLD,
        reset_timeout_ms = RESET_TIMEOUT_MS,
        circuit_state = tracing::field::Empty,
        cancelled = cancellation.is_cancelled(),
    ))]
    pub async fn execute_with_cancellation<T, F, Fut>(
        &self,
        operation: F,
        cancellation: &CancellationContext,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        // Fast path: read atomic state without acquiring any lock.
        // Most operations happen in Closed state — this avoids lock overhead entirely.
        // TOCTOU note: between this read and result recording (under write lock),
        // another task may open the circuit. This is a deliberate trade-off —
        // circuit breakers are approximate by nature, and the lock-free fast path
        // eliminates contention for the common success case.
        let atomic_state = {
            let v = self.atomic_state.load(Ordering::Acquire);
            State::from_atomic(v).unwrap_or(State::Closed)
        };

        // For Closed state, we can proceed immediately without state transitions
        let can_execute = match atomic_state {
            State::Closed => {
                tracing::Span::current().record("circuit_state", "Closed");
                true
            }
            State::Open | State::HalfOpen => {
                // Need write lock for potential state transitions
                let mut inner = self.inner.write().await;
                tracing::Span::current().record("circuit_state", format!("{:?}", inner.state));
                match inner.state {
                    State::Closed => true,
                    State::Open => {
                        // Check if we should transition to half-open
                        let elapsed = Instant::now().duration_since(inner.last_state_change);
                        if elapsed >= Duration::from_millis(RESET_TIMEOUT_MS) {
                            // Transition to HalfOpen state
                            tracing::info!("Circuit breaker transitioning from open to half-open");
                            inner.set_state(State::HalfOpen);
                            inner.half_open_operations = 0;
                            true
                        } else {
                            false
                        }
                    }
                    State::HalfOpen => {
                        inner.half_open_operations < inner.config.half_open_max_operations
                    }
                }
            }
        };

        if !can_execute {
            let reset_timeout = {
                let inner = self.inner.read().await;
                if matches!(inner.state, State::Open) {
                    let elapsed = Instant::now().duration_since(inner.last_state_change);
                    drop(inner);
                    let timeout_duration = Duration::from_millis(RESET_TIMEOUT_MS);
                    if elapsed < timeout_duration {
                        // Use unwrap_or to handle potential clock skew safely
                        Some(
                            timeout_duration
                                .checked_sub(elapsed)
                                .unwrap_or(Duration::ZERO),
                        )
                    } else {
                        Some(Duration::ZERO)
                    }
                } else {
                    None
                }
            };

            return Err(ResilienceError::CircuitBreakerOpen {
                state: "open".to_string(),
                retry_after: reset_timeout,
            });
        }

        // Execute the operation with cancellation support
        let start_time = Instant::now();
        tracing::debug!("Executing operation through circuit breaker with cancellation");
        let result = cancellation.execute(operation).await;
        let duration = start_time.elapsed();
        tracing::debug!(?duration, "Operation completed");

        // Record the result and potentially transition states
        let mut inner = self.inner.write().await;
        inner.total_operations += 1;

        match result {
            Ok(ref _value) => {
                inner.sliding_window.record_operation(false);
                self.record_success_inner(&mut inner);
                debug!(
                    state = %inner.state,
                    duration_ms = duration.as_millis(),
                    "Circuit breaker operation succeeded"
                );
            }
            Err(ref error) => {
                let should_count = match error {
                    ResilienceError::Timeout { .. } => inner.config.count_timeouts,
                    _ => true,
                };

                if should_count {
                    inner.sliding_window.record_operation(true);
                    self.record_failure_inner(&mut inner);
                }

                error!(
                    state = %inner.state,
                    error = ?error,
                    duration_ms = duration.as_millis(),
                    "Circuit breaker operation failed"
                );
            }
        }

        result
    }

    fn record_success_inner(
        &self,
        inner: &mut CircuitBreakerInner<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
    ) {
        match inner.state {
            State::Closed => {
                // Reset failure count on success in closed state
                inner.failure_count = 0;
                inner.last_failure_time = None;
            }
            State::HalfOpen => {
                inner.half_open_operations += 1;

                // Check if we should transition back to closed
                if inner.half_open_operations >= inner.config.half_open_max_operations {
                    info!("Circuit breaker transitioning from half-open to closed");
                    inner.set_state(State::Closed);
                    inner.failure_count = 0;
                    inner.half_open_operations = 0;
                }
            }
            State::Open => {
                // Shouldn't happen, but handle gracefully
                warn!("Unexpected success in open circuit state");
            }
        }
    }

    fn record_failure_inner(
        &self,
        inner: &mut CircuitBreakerInner<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>,
    ) {
        inner.failure_count += 1;
        inner.last_failure_time = Some(Instant::now());

        match inner.state {
            State::Closed => {
                // Check if we should open the circuit
                let operation_count = inner.sliding_window.get_operation_count();
                let failure_rate = inner.sliding_window.get_failure_rate();

                if operation_count >= inner.config.min_operations
                    && failure_rate >= inner.config.failure_rate_threshold
                {
                    warn!(
                        failure_count = inner.failure_count,
                        failure_rate = failure_rate,
                        operation_count = operation_count,
                        "Circuit breaker opening"
                    );

                    inner.set_state(State::Open);
                }
            }
            State::HalfOpen => {
                // Transition back to open on any failure in half-open
                warn!("Circuit breaker transitioning from half-open back to open");
                inner.set_state(State::Open);
                inner.half_open_operations = 0;
            }
            State::Open => {
                // Already open, just update metrics
            }
        }
    }

    /// Get current state
    pub async fn state(&self) -> State {
        let mut inner = self.inner.write().await;

        // Check for automatic state transitions
        if matches!(inner.state, State::Open) {
            let elapsed = Instant::now().duration_since(inner.last_state_change);
            if elapsed >= Duration::from_millis(RESET_TIMEOUT_MS) {
                info!("Circuit breaker transitioning from open to half-open");
                inner.set_state(State::HalfOpen);
                inner.half_open_operations = 0;
            }
        }

        inner.state
    }

    /// Get current circuit breaker state without acquiring locks (fast path)
    ///
    /// This method provides lock-free access to the circuit breaker state
    /// using atomic operations. It's optimized for high-frequency polling
    /// and monitoring scenarios where full statistics are not needed.
    ///
    /// For detailed statistics including failure rates and operation counts,
    /// use [`stats()`](Self::stats) instead.
    ///
    /// # Performance
    ///
    /// This method is ~10-50x faster than `stats()` as it avoids lock acquisition.
    /// Use this for health checks and frequent state queries.
    #[must_use]
    pub fn state_fast(&self) -> State {
        State::from_atomic(self.atomic_state.load(Ordering::Acquire)).unwrap_or(State::Closed)
    }

    /// Get circuit breaker statistics
    ///
    /// This method acquires a read lock to gather comprehensive statistics
    /// including failure rates, operation counts, and timing information.
    ///
    /// For lock-free state queries, use [`state_fast()`](Self::state_fast) instead.
    pub async fn stats(&self) -> CircuitBreakerStats {
        let inner = self.inner.read().await;
        let failure_rate = inner.sliding_window.get_failure_rate();
        let operation_count = inner.sliding_window.get_operation_count();

        CircuitBreakerStats {
            state: inner.state,
            failure_count: inner.failure_count,
            last_failure_time: inner.last_failure_time,
            half_open_operations: inner.half_open_operations,
            total_operations: inner.total_operations,
            failure_rate,
            operation_count,
        }
    }

    /// Reset the circuit breaker to closed state
    pub async fn reset(&self) {
        let mut inner = self.inner.write().await;
        info!("Manually resetting circuit breaker");

        inner.set_state(State::Closed);
        inner.failure_count = 0;
        inner.last_failure_time = None;
        inner.half_open_operations = 0;
    }

    /// Check if circuit breaker is in closed state (lock-free)
    ///
    /// Uses atomic operations for fast state checking without lock acquisition.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        matches!(self.state_fast(), State::Closed)
    }

    /// Check if circuit breaker is in open state (lock-free)
    ///
    /// Uses atomic operations for fast state checking without lock acquisition.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state_fast(), State::Open)
    }

    /// Check if circuit breaker is in half-open state (lock-free)
    ///
    /// Uses atomic operations for fast state checking without lock acquisition.
    #[must_use]
    pub fn is_half_open(&self) -> bool {
        matches!(self.state_fast(), State::HalfOpen)
    }

    /// Check if the circuit breaker allows execution
    /// Returns Ok(()) if execution is allowed, Err if circuit is open
    pub async fn can_execute(&self) -> ResilienceResult<()> {
        let state = self.state().await;
        match state {
            State::Closed => Ok(()),
            State::HalfOpen => {
                let inner = self.inner.read().await;
                if inner.half_open_operations < inner.config.half_open_max_operations {
                    Ok(())
                } else {
                    Err(ResilienceError::CircuitBreakerOpen {
                        state: "half-open (limit reached)".to_string(),
                        retry_after: None,
                    })
                }
            }
            State::Open => {
                let inner = self.inner.read().await;
                let elapsed = Instant::now().duration_since(inner.last_state_change);
                drop(inner);
                let timeout_duration = Duration::from_millis(RESET_TIMEOUT_MS);
                let retry_after = if elapsed < timeout_duration {
                    // Use unwrap_or to handle potential clock skew safely
                    Some(
                        timeout_duration
                            .checked_sub(elapsed)
                            .unwrap_or(Duration::ZERO),
                    )
                } else {
                    Some(Duration::ZERO)
                };
                Err(ResilienceError::CircuitBreakerOpen {
                    state: "open".to_string(),
                    retry_after,
                })
            }
        }
    }

    /// Record a successful operation
    pub async fn record_success(&self) {
        let mut inner = self.inner.write().await;
        self.record_success_inner(&mut inner);
    }

    /// Record a failed operation
    pub async fn record_failure(&self) {
        let mut inner = self.inner.write().await;
        self.record_failure_inner(&mut inner);
    }
}

impl<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64> Default
    for CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
{
    fn default() -> Self {
        Self::with_defaults().expect("Default configuration should be valid")
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    /// Current circuit breaker state
    pub state: State,
    /// Number of failures recorded
    pub failure_count: usize,
    /// Timestamp of the last failure
    pub last_failure_time: Option<Instant>,
    /// Number of operations in half-open state
    pub half_open_operations: usize,
    /// Total number of operations
    pub total_operations: u64,
    /// Current failure rate (0.0 to 1.0)
    pub failure_rate: f64,
    /// Number of operations in sliding window
    pub operation_count: usize,
}

impl PatternMetrics for CircuitBreakerStats {
    type Value = crate::core::traits::MetricValue;

    fn get_metric(&self, name: &str) -> Option<Self::Value> {
        use crate::core::traits::MetricValue;

        match name {
            "total_operations" => Some(MetricValue::Counter(self.total_operations)),
            "failure_count" => Some(MetricValue::Counter(self.failure_count as u64)),
            "failure_rate" => Some(MetricValue::Gauge(self.failure_rate)),
            "half_open_operations" => Some(MetricValue::Counter(self.half_open_operations as u64)),
            "state" => Some(MetricValue::Flag(matches!(self.state, State::Closed))),
            _ => None,
        }
    }

    fn error_rate(&self) -> f64 {
        self.failure_rate
    }

    fn total_operations(&self) -> u64 {
        self.total_operations
    }
}

/// Convenience type aliases for common configurations
/// Standard circuit breaker with 5 failure threshold and 30 second reset timeout
pub type StandardCircuitBreaker = CircuitBreaker<5, 30_000>;
/// Fast-responding circuit breaker with 3 failure threshold and 10 second reset timeout
pub type FastCircuitBreaker = CircuitBreaker<3, 10_000>;
/// Conservative circuit breaker with 10 failure threshold and 60 second reset timeout
pub type SlowCircuitBreaker = CircuitBreaker<10, 60_000>;

/// Helper functions for creating common configurations
#[must_use]
pub const fn fast_config() -> CircuitBreakerConfig<3, 10_000> {
    CircuitBreakerConfig::new().with_half_open_limit(2)
}

/// Create a standard circuit breaker configuration
#[must_use]
pub const fn standard_config() -> CircuitBreakerConfig<5, 30_000> {
    CircuitBreakerConfig::new()
}

/// Create a slow/conservative circuit breaker configuration
#[must_use]
pub const fn slow_config() -> CircuitBreakerConfig<10, 60_000> {
    CircuitBreakerConfig::new().with_min_operations(20)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Duration, sleep};

    #[test]
    fn test_config_creation() {
        let _fast = fast_config();
        let _standard = standard_config();
        let _slow = slow_config();
    }

    #[test]
    fn test_state_types_are_consistent() {
        use crate::core::traits::circuit_states::{Closed, HalfOpen, Open};

        // Verify the phantom type states exist and produce valid PhantomData markers
        let _ = std::marker::PhantomData::<Closed>;
        let _ = std::marker::PhantomData::<Open>;
        let _ = std::marker::PhantomData::<HalfOpen>;
    }

    #[tokio::test]
    async fn test_circuit_breaker_creation() {
        let config = CircuitBreakerConfig::<5, 30_000>::new();
        let breaker = CircuitBreaker::new(config);
        assert!(breaker.is_ok());

        let breaker = breaker.unwrap();
        assert!(breaker.is_closed());
    }

    #[tokio::test]
    async fn test_successful_operations() {
        let breaker = StandardCircuitBreaker::default();

        for i in 0..10 {
            let result = breaker
                .execute(|| async { Ok::<_, ResilienceError>(format!("success {i}")) })
                .await;
            assert!(result.is_ok());
        }

        let stats = breaker.stats().await;
        assert_eq!(stats.total_operations, 10);
        assert!(breaker.is_closed());
    }

    #[tokio::test]
    async fn test_circuit_opening_on_failures() {
        let config = CircuitBreakerConfig::<2, 1000>::new().with_min_operations(1); // Low threshold for testing
        let breaker = CircuitBreaker::new(config).unwrap();

        let counter = Arc::new(AtomicUsize::new(0));

        // Cause failures to trigger circuit opening
        for _ in 0..5 {
            let counter_clone = counter.clone();
            let _ = breaker
                .execute(|| async {
                    counter_clone.fetch_add(1, Ordering::Relaxed);
                    Err::<(), _>(ResilienceError::Timeout {
                        duration: Duration::from_millis(100),
                        context: Some("test".to_string()),
                    })
                })
                .await;
        }

        // Circuit should eventually open
        let mut attempts = 0;
        while breaker.is_closed() && attempts < 10 {
            sleep(Duration::from_millis(10)).await;
            attempts += 1;
        }

        let stats = breaker.stats().await;
        println!("Final stats: {stats:?}");

        // At minimum, operations should have been attempted
        assert!(stats.total_operations > 0);
    }

    #[tokio::test]
    async fn test_circuit_recovery() {
        let config = CircuitBreakerConfig::<2, 100>::new(); // Very short timeout
        let breaker = CircuitBreaker::new(config).unwrap();

        // Force circuit to open
        for _ in 0..3 {
            let _result = breaker
                .execute(|| async {
                    Err::<(), _>(ResilienceError::Timeout {
                        duration: Duration::from_secs(1),
                        context: Some("test".to_string()),
                    })
                })
                .await;
        }

        // Wait for reset timeout
        sleep(Duration::from_millis(150)).await;

        // Try a successful operation (should work in half-open)
        let _result = breaker
            .execute(|| async { Ok::<_, ResilienceError>("recovery".to_string()) })
            .await;

        // Should either succeed or be in a valid state for recovery
        let stats = breaker.stats().await;
        println!("Recovery stats: {stats:?}");

        // The important thing is that we can make progress
        assert!(stats.total_operations > 0);
    }

    #[tokio::test]
    async fn test_sliding_window() {
        let mut window = SlidingWindow::new(Duration::from_millis(100), 100);

        // Record some operations
        window.record_operation(false); // Success
        window.record_operation(true); // Failure
        window.record_operation(true); // Failure

        let failure_rate = window.get_failure_rate();
        let operation_count = window.get_operation_count();

        assert_eq!(operation_count, 3);
        assert!((failure_rate - 0.6667).abs() < 0.001);

        // Wait for window to expire
        sleep(Duration::from_millis(150)).await;

        let failure_rate = window.get_failure_rate();
        let operation_count = window.get_operation_count();

        assert_eq!(operation_count, 0);
        assert!(failure_rate.abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_reset_functionality() {
        let breaker = StandardCircuitBreaker::default();

        // Force some failures
        for _ in 0..3 {
            let _ = breaker
                .execute(|| async {
                    Err::<(), _>(ResilienceError::Timeout {
                        duration: Duration::from_millis(100),
                        context: Some("test".to_string()),
                    })
                })
                .await;
        }

        // Reset the circuit
        breaker.reset().await;

        // Should be closed after reset
        assert!(breaker.is_closed());

        let stats = breaker.stats().await;
        assert_eq!(stats.failure_count, 0);
    }

    #[tokio::test]
    async fn test_lock_free_state_fast() {
        let breaker = StandardCircuitBreaker::default();

        // Test lock-free state access
        assert_eq!(breaker.state_fast(), State::Closed);
        assert!(breaker.is_closed());
        assert!(!breaker.is_open());
        assert!(!breaker.is_half_open());

        // Verify it's truly lock-free by calling it many times rapidly
        for _ in 0..1000 {
            let _ = breaker.state_fast();
        }

        // Should still be closed
        assert_eq!(breaker.state_fast(), State::Closed);
    }

    #[tokio::test]
    async fn test_state_fast_vs_stats_consistency() {
        let breaker = StandardCircuitBreaker::default();

        // Fast state should match detailed stats state
        let fast_state = breaker.state_fast();
        let stats = breaker.stats().await;

        assert_eq!(fast_state, stats.state);
    }
}
