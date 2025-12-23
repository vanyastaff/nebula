//! Type-safe retry strategies using advanced Rust type system features
//!
//! This module provides compile-time safe retry strategies using:
//! - Const generics for attempt limits and timing
//! - Zero-cost abstractions with phantom types
//! - Type-level programming for backoff policies
//! - HRTB for flexible operation handling
//! - GATs for better async trait design

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::core::{
    ResilienceError, ResilienceResult,
    config::{ConfigError, ConfigResult},
    traits::PatternMetrics,
};

/// Sealed trait for backoff policies to prevent external implementations
mod sealed {
    pub trait SealedBackoff {}
}

/// Trait for compile-time safe backoff policies
pub trait BackoffPolicy:
    sealed::SealedBackoff
    + Send
    + Sync
    + fmt::Debug
    + Clone
    + serde::Serialize
    + for<'de> serde::Deserialize<'de>
    + 'static
{
    /// Calculate delay for given attempt (0-indexed)
    fn calculate_delay(&self, attempt: usize) -> Duration;

    /// Maximum theoretical delay this policy can produce
    fn max_delay(&self) -> Duration;

    /// Check if policy parameters are valid at compile time
    #[must_use] 
    fn is_valid() -> bool {
        true
    }

    /// Get policy name for observability
    fn policy_name(&self) -> &'static str;
}

/// Fixed delay backoff policy with compile-time validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedDelay<const DELAY_MS: u64> {
    _marker: PhantomData<()>,
}

impl<const DELAY_MS: u64> Default for FixedDelay<DELAY_MS> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<const DELAY_MS: u64> sealed::SealedBackoff for FixedDelay<DELAY_MS> {}

impl<const DELAY_MS: u64> BackoffPolicy for FixedDelay<DELAY_MS> {
    fn calculate_delay(&self, _attempt: usize) -> Duration {
        Duration::from_millis(DELAY_MS)
    }

    fn max_delay(&self) -> Duration {
        Duration::from_millis(DELAY_MS)
    }

    fn is_valid() -> bool {
        DELAY_MS > 0 && DELAY_MS < 300_000 // Max 5 minutes
    }

    fn policy_name(&self) -> &'static str {
        "FixedDelay"
    }
}

/// Linear backoff policy: delay = `base_delay` * (attempt + 1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearBackoff<const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64 = 30_000> {
    _marker: PhantomData<()>,
}

impl<const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64> Default
    for LinearBackoff<BASE_DELAY_MS, MAX_DELAY_MS>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64> sealed::SealedBackoff
    for LinearBackoff<BASE_DELAY_MS, MAX_DELAY_MS>
{
}

impl<const BASE_DELAY_MS: u64, const MAX_DELAY_MS: u64> BackoffPolicy
    for LinearBackoff<BASE_DELAY_MS, MAX_DELAY_MS>
{
    fn calculate_delay(&self, attempt: usize) -> Duration {
        let linear_delay = BASE_DELAY_MS.saturating_mul(attempt as u64 + 1);
        Duration::from_millis(linear_delay.min(MAX_DELAY_MS))
    }

    fn max_delay(&self) -> Duration {
        Duration::from_millis(MAX_DELAY_MS)
    }

    fn is_valid() -> bool {
        BASE_DELAY_MS > 0 && MAX_DELAY_MS >= BASE_DELAY_MS && MAX_DELAY_MS < 300_000
    }

    fn policy_name(&self) -> &'static str {
        "LinearBackoff"
    }
}

/// Exponential backoff policy: delay = `base_delay` * multiplier^attempt
/// Multiplier is encoded as `MULTIPLIER_X10` to avoid floating point in const generics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExponentialBackoff<
    const BASE_DELAY_MS: u64,
    const MULTIPLIER_X10: u64 = 20,
    const MAX_DELAY_MS: u64 = 30_000,
> {
    _marker: PhantomData<()>,
}

impl<const BASE_DELAY_MS: u64, const MULTIPLIER_X10: u64, const MAX_DELAY_MS: u64> Default
    for ExponentialBackoff<BASE_DELAY_MS, MULTIPLIER_X10, MAX_DELAY_MS>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<const BASE_DELAY_MS: u64, const MULTIPLIER_X10: u64, const MAX_DELAY_MS: u64>
    sealed::SealedBackoff for ExponentialBackoff<BASE_DELAY_MS, MULTIPLIER_X10, MAX_DELAY_MS>
{
}

impl<const BASE_DELAY_MS: u64, const MULTIPLIER_X10: u64, const MAX_DELAY_MS: u64> BackoffPolicy
    for ExponentialBackoff<BASE_DELAY_MS, MULTIPLIER_X10, MAX_DELAY_MS>
{
    fn calculate_delay(&self, attempt: usize) -> Duration {
        let multiplier = MULTIPLIER_X10 as f64 / 10.0;
        let exp_delay = (BASE_DELAY_MS as f64) * multiplier.powi(attempt as i32);
        let capped_delay = (exp_delay as u64).min(MAX_DELAY_MS);
        Duration::from_millis(capped_delay)
    }

    fn max_delay(&self) -> Duration {
        Duration::from_millis(MAX_DELAY_MS)
    }

    fn is_valid() -> bool {
        BASE_DELAY_MS > 0
            && MULTIPLIER_X10 >= 10  // At least 1.0 multiplier
            && MULTIPLIER_X10 <= 100 // At most 10.0 multiplier
            && MAX_DELAY_MS >= BASE_DELAY_MS
            && MAX_DELAY_MS < 300_000
    }

    fn policy_name(&self) -> &'static str {
        "ExponentialBackoff"
    }
}

/// Custom backoff with predefined delays
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomBackoff {
    delays: Vec<Duration>,
    default_delay: Duration,
}

impl CustomBackoff {
    /// Create new custom backoff with delays
    #[must_use] 
    pub fn new(delays: Vec<Duration>, default_delay: Duration) -> Self {
        Self {
            delays,
            default_delay,
        }
    }

    /// Create from millisecond values
    #[must_use] 
    pub fn from_millis(delays: &[u64], default_ms: u64) -> Self {
        let delays = delays.iter().map(|&ms| Duration::from_millis(ms)).collect();
        Self::new(delays, Duration::from_millis(default_ms))
    }
}

impl sealed::SealedBackoff for CustomBackoff {}

impl BackoffPolicy for CustomBackoff {
    fn calculate_delay(&self, attempt: usize) -> Duration {
        self.delays
            .get(attempt)
            .copied()
            .unwrap_or(self.default_delay)
    }

    fn max_delay(&self) -> Duration {
        self.delays
            .iter()
            .max()
            .copied()
            .unwrap_or(self.default_delay)
            .max(self.default_delay)
    }

    fn policy_name(&self) -> &'static str {
        "CustomBackoff"
    }
}

/// Jitter policy for avoiding thundering herd with zero-cost abstractions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JitterPolicy {
    /// No jitter - use calculated delay exactly
    None,
    /// Full jitter: random(0, `calculated_delay`)
    Full,
    /// Equal jitter: `calculated_delay/2` + random(0, `calculated_delay/2`)
    Equal,
    /// Decorrelated jitter: more sophisticated algorithm
    Decorrelated,
}

impl JitterPolicy {
    /// Apply jitter to a delay using fast RNG
    #[must_use] 
    pub fn apply(self, delay: Duration, previous_delay: Option<Duration>) -> Duration {
        match self {
            Self::None => delay,
            Self::Full => {
                let millis = delay.as_millis() as u64;
                if millis == 0 {
                    return delay;
                }
                Duration::from_millis(fastrand::u64(0..=millis))
            }
            Self::Equal => {
                let millis = delay.as_millis() as u64;
                let half = millis / 2;
                let jitter = if half > 0 { fastrand::u64(0..=half) } else { 0 };
                Duration::from_millis(half + jitter)
            }
            Self::Decorrelated => {
                let base = delay.as_millis() as u64;
                if let Some(prev) = previous_delay {
                    let prev_millis = prev.as_millis() as u64;
                    let upper = (prev_millis * 3).max(base);
                    Duration::from_millis(fastrand::u64(base..=upper))
                } else {
                    // First attempt, use equal jitter
                    Self::Equal.apply(delay, None)
                }
            }
        }
    }
}

/// Type-safe retry condition with compile-time error type specification
pub trait RetryCondition<E>:
    Send + Sync + fmt::Debug + Clone + serde::Serialize + for<'de> serde::Deserialize<'de>
{
    /// Check if error should trigger retry
    fn should_retry(&self, error: &E, attempt: usize, elapsed: Duration) -> bool;

    /// Check if error is terminal (never retry)
    fn is_terminal(&self, error: &E) -> bool;

    /// Get custom delay override for this specific error
    fn custom_delay(&self, error: &E, attempt: usize) -> Option<Duration> {
        let _ = (error, attempt);
        None
    }

    /// Get condition name for observability
    fn condition_name(&self) -> &'static str;
}

/// Conservative retry condition - only retries known safe errors
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ConservativeCondition<const MAX_ATTEMPTS: usize = 3> {
    _marker: PhantomData<()>,
}

impl<const MAX_ATTEMPTS: usize> ConservativeCondition<MAX_ATTEMPTS> {
    /// Create a new conservative retry condition.
    #[must_use] 
    pub const fn new() -> Self {
        assert!(MAX_ATTEMPTS > 0, "Max attempts must be positive");
        Self {
            _marker: PhantomData,
        }
    }
}

// Implementation for ResilienceError using pattern matching instead of string formatting
impl<const MAX_ATTEMPTS: usize> RetryCondition<crate::core::ResilienceError>
    for ConservativeCondition<MAX_ATTEMPTS>
{
    fn should_retry(
        &self,
        error: &crate::core::ResilienceError,
        attempt: usize,
        _elapsed: Duration,
    ) -> bool {
        if attempt >= MAX_ATTEMPTS {
            return false;
        }

        // Use ResilienceError's built-in retryability check
        error.is_retryable()
    }

    fn is_terminal(&self, error: &crate::core::ResilienceError) -> bool {
        // Use ResilienceError's built-in terminal check
        error.is_terminal()
    }

    fn custom_delay(&self, error: &crate::core::ResilienceError, _attempt: usize) -> Option<Duration> {
        // Use ResilienceError's retry_after hint
        error.retry_after()
    }

    fn condition_name(&self) -> &'static str {
        "Conservative"
    }
}

/// Aggressive retry condition - retries most errors
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AggressiveCondition<const MAX_ATTEMPTS: usize = 5> {
    _marker: PhantomData<()>,
}

impl<const MAX_ATTEMPTS: usize> AggressiveCondition<MAX_ATTEMPTS> {
    /// Create a new aggressive retry condition.
    #[must_use] 
    pub const fn new() -> Self {
        assert!(MAX_ATTEMPTS > 0, "Max attempts must be positive");
        Self {
            _marker: PhantomData,
        }
    }
}

impl<E, const MAX_ATTEMPTS: usize> RetryCondition<E> for AggressiveCondition<MAX_ATTEMPTS>
where
    E: fmt::Debug,
{
    fn should_retry(&self, error: &E, attempt: usize, _elapsed: Duration) -> bool {
        if attempt >= MAX_ATTEMPTS {
            return false;
        }
        !self.is_terminal(error)
    }

    fn is_terminal(&self, error: &E) -> bool {
        let error_str = format!("{error:?}");
        error_str.contains("NotFound")
            || error_str.contains("Unauthorized")
            || error_str.contains("InvalidInput")
            || error_str.contains("PermissionDenied")
            || error_str.contains("ParseError")
    }

    fn condition_name(&self) -> &'static str {
        "Aggressive"
    }
}

/// Time-based retry condition with deadline enforcement
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimeBasedCondition<const MAX_DURATION_MS: u64 = 30_000> {
    max_attempts: usize,
    _marker: PhantomData<()>,
}

impl<const MAX_DURATION_MS: u64> TimeBasedCondition<MAX_DURATION_MS> {
    /// Create a new time-based condition with the given maximum attempts.
    #[must_use] 
    pub const fn new(max_attempts: usize) -> Self {
        assert!(max_attempts > 0, "Max attempts must be positive");
        assert!(MAX_DURATION_MS > 0, "Max duration must be positive");
        Self {
            max_attempts,
            _marker: PhantomData,
        }
    }
}

impl<const MAX_DURATION_MS: u64> Default for TimeBasedCondition<MAX_DURATION_MS> {
    fn default() -> Self {
        Self::new(5)
    }
}

impl<E, const MAX_DURATION_MS: u64> RetryCondition<E> for TimeBasedCondition<MAX_DURATION_MS>
where
    E: fmt::Debug,
{
    fn should_retry(&self, error: &E, attempt: usize, elapsed: Duration) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }

        if elapsed >= Duration::from_millis(MAX_DURATION_MS) {
            return false;
        }

        !self.is_terminal(error)
    }

    fn is_terminal(&self, error: &E) -> bool {
        let error_str = format!("{error:?}");
        error_str.contains("NotFound")
            || error_str.contains("Unauthorized")
            || error_str.contains("InvalidInput")
    }

    fn condition_name(&self) -> &'static str {
        "TimeBased"
    }
}

/// Retry statistics with zero-cost metric collection
#[derive(Debug, Clone)]
pub struct RetryStats {
    /// Total attempts made (including initial)
    pub total_attempts: usize,
    /// Total time spent in retry loop
    pub total_duration: Duration,
    /// Whether the final result was successful
    pub succeeded: bool,
    /// Delays used between attempts
    pub attempt_delays: Vec<Duration>,
    /// Duration of each attempt
    pub attempt_durations: Vec<Duration>,
    /// Backoff policy used
    pub policy_name: String,
    /// Condition used
    pub condition_name: String,
}

impl PatternMetrics for RetryStats {
    type Value = crate::core::traits::MetricValue;

    fn get_metric(&self, name: &str) -> Option<Self::Value> {
        use crate::core::traits::MetricValue;

        match name {
            "total_attempts" => Some(MetricValue::Counter(self.total_attempts as u64)),
            "total_duration" => Some(MetricValue::Duration(self.total_duration)),
            "succeeded" => Some(MetricValue::Flag(self.succeeded)),
            "policy_name" => Some(MetricValue::Flag(true)), // Could be string in future
            "avg_delay" => {
                if self.attempt_delays.is_empty() {
                    Some(MetricValue::Duration(Duration::ZERO))
                } else {
                    let avg_ms = self
                        .attempt_delays
                        .iter()
                        .map(|d| d.as_millis() as u64)
                        .sum::<u64>()
                        / self.attempt_delays.len() as u64;
                    Some(MetricValue::Duration(Duration::from_millis(avg_ms)))
                }
            }
            "avg_attempt_duration" => {
                if self.attempt_durations.is_empty() {
                    Some(MetricValue::Duration(Duration::ZERO))
                } else {
                    let avg_ms = self
                        .attempt_durations
                        .iter()
                        .map(|d| d.as_millis() as u64)
                        .sum::<u64>()
                        / self.attempt_durations.len() as u64;
                    Some(MetricValue::Duration(Duration::from_millis(avg_ms)))
                }
            }
            _ => None,
        }
    }

    fn error_rate(&self) -> f64 {
        if self.succeeded { 0.0 } else { 1.0 }
    }

    fn total_operations(&self) -> u64 {
        self.total_attempts as u64
    }
}

/// Type-safe retry configuration with const generics
#[derive(Debug, Clone)]
pub struct RetryConfig<B: BackoffPolicy, C> {
    /// Backoff policy
    pub backoff: B,
    /// Retry condition
    pub condition: C,
    /// Jitter policy
    pub jitter: JitterPolicy,
    /// Maximum total duration for all retries
    pub max_total_duration: Option<Duration>,
}

impl<B: BackoffPolicy, C> RetryConfig<B, C> {
    /// Create new retry configuration
    pub fn new(backoff: B, condition: C) -> Self {
        Self {
            backoff,
            condition,
            jitter: JitterPolicy::Equal,
            max_total_duration: None,
        }
    }

    /// Set jitter policy
    pub fn with_jitter(mut self, jitter: JitterPolicy) -> Self {
        self.jitter = jitter;
        self
    }

    /// Set maximum total duration
    pub fn with_max_duration(mut self, duration: Duration) -> Self {
        self.max_total_duration = Some(duration);
        self
    }
}

impl<B: BackoffPolicy, C> RetryConfig<B, C> {
    /// Validate the configuration
    pub fn validate(&self) -> ConfigResult<()> {
        if !B::is_valid() {
            return Err(ConfigError::validation(
                "Invalid backoff policy configuration",
            ));
        }

        if let Some(max_duration) = self.max_total_duration
            && max_duration.is_zero() {
                return Err(ConfigError::validation(
                    "max_total_duration must be positive",
                ));
            }

        Ok(())
    }
}

/// Type-safe retry strategy with compile-time guarantees
pub struct RetryStrategy<B: BackoffPolicy, C> {
    config: RetryConfig<B, C>,
}

impl<B: BackoffPolicy, C> RetryStrategy<B, C> {
    /// Create new retry strategy
    pub fn new(config: RetryConfig<B, C>) -> ConfigResult<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Create with backoff and condition
    pub fn with_policy(backoff: B, condition: C) -> ConfigResult<Self> {
        let config = RetryConfig::new(backoff, condition);
        Self::new(config)
    }

    /// Execute operation with retry logic using HRTB
    pub async fn execute<T, E, F, Fut>(&self, mut operation: F) -> Result<(T, RetryStats), E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: fmt::Debug,
        C: RetryCondition<E>,
    {
        let start_time = Instant::now();
        let mut attempt = 0;
        let mut attempt_delays = Vec::new();
        let mut attempt_durations = Vec::new();
        let mut previous_delay = None;

        loop {
            let attempt_start = Instant::now();

            debug!(
                attempt = attempt + 1,
                policy = self.config.backoff.policy_name(),
                condition = self.config.condition.condition_name(),
                "Starting retry attempt"
            );

            let result = operation().await;
            let attempt_duration = attempt_start.elapsed();
            attempt_durations.push(attempt_duration);

            match result {
                Ok(value) => {
                    let stats = RetryStats {
                        total_attempts: attempt + 1,
                        total_duration: start_time.elapsed(),
                        succeeded: true,
                        attempt_delays,
                        attempt_durations,
                        policy_name: self.config.backoff.policy_name().to_string(),
                        condition_name: self.config.condition.condition_name().to_string(),
                    };

                    info!(
                        attempts = stats.total_attempts,
                        duration_ms = stats.total_duration.as_millis(),
                        "Retry succeeded"
                    );

                    return Ok((value, stats));
                }
                Err(error) => {
                    let elapsed = start_time.elapsed();

                    // Check if we should retry
                    let should_retry = self.config.condition.should_retry(&error, attempt, elapsed)
                        && !self.config.condition.is_terminal(&error);

                    // Check total duration limit
                    if let Some(max_duration) = self.config.max_total_duration
                        && elapsed >= max_duration {
                            warn!(
                                attempts = attempt + 1,
                                elapsed_ms = elapsed.as_millis(),
                                max_ms = max_duration.as_millis(),
                                "Retry failed: maximum duration exceeded"
                            );

                            let _stats = RetryStats {
                                total_attempts: attempt + 1,
                                total_duration: elapsed,
                                succeeded: false,
                                attempt_delays,
                                attempt_durations,
                                policy_name: self.config.backoff.policy_name().to_string(),
                                condition_name: self.config.condition.condition_name().to_string(),
                            };
                            return Err(error);
                        }

                    if !should_retry {
                        warn!(
                            attempts = attempt + 1,
                            error = ?error,
                            "Retry failed: no more attempts"
                        );

                        let _stats = RetryStats {
                            total_attempts: attempt + 1,
                            total_duration: elapsed,
                            succeeded: false,
                            attempt_delays,
                            attempt_durations,
                            policy_name: self.config.backoff.policy_name().to_string(),
                            condition_name: self.config.condition.condition_name().to_string(),
                        };
                        return Err(error);
                    }

                    // Calculate delay for next attempt
                    let base_delay = if let Some(custom_delay) =
                        self.config.condition.custom_delay(&error, attempt)
                    {
                        custom_delay
                    } else {
                        self.config.backoff.calculate_delay(attempt)
                    };

                    let jittered_delay = self.config.jitter.apply(base_delay, previous_delay);
                    attempt_delays.push(jittered_delay);
                    previous_delay = Some(jittered_delay);
                    attempt += 1;

                    debug!(
                        attempt = attempt,
                        delay_ms = jittered_delay.as_millis(),
                        error = ?error,
                        "Retrying after delay"
                    );

                    // Sleep before next attempt
                    sleep(jittered_delay).await;
                }
            }
        }
    }

    /// Execute with `ResilienceError` handling
    pub async fn execute_resilient<T, F, Fut>(
        &self,
        operation: F,
    ) -> ResilienceResult<(T, RetryStats)>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        C: RetryCondition<ResilienceError>,
    {
        match self.execute(operation).await {
            Ok(result) => Ok(result),
            Err(error) => Err(error),
        }
    }

    /// Get configuration
    pub fn config(&self) -> &RetryConfig<B, C> {
        &self.config
    }
}

/// Convenience type aliases for common retry strategies
pub type StandardRetry = RetryStrategy<ExponentialBackoff<100, 20>, ConservativeCondition<3>>;
/// Quick retry strategy with short delays and few attempts.
pub type QuickRetry = RetryStrategy<FixedDelay<50>, ConservativeCondition<2>>;
/// Aggressive retry strategy with many attempts.
pub type AggressiveRetry = RetryStrategy<ExponentialBackoff<50, 15>, AggressiveCondition<5>>;
/// Time-constrained retry strategy with linear backoff.
pub type TimeConstrainedRetry = RetryStrategy<LinearBackoff<100, 5000>, TimeBasedCondition<10_000>>;

/// Helper functions for creating common retry strategies
pub fn exponential_retry<const MAX_ATTEMPTS: usize>()
-> ConfigResult<RetryStrategy<ExponentialBackoff<100, 20>, ConservativeCondition<MAX_ATTEMPTS>>> {
    RetryStrategy::with_policy(ExponentialBackoff::default(), ConservativeCondition::new())
}

/// Create a fixed-delay retry strategy.
pub fn fixed_retry<const DELAY_MS: u64, const MAX_ATTEMPTS: usize>()
-> ConfigResult<RetryStrategy<FixedDelay<DELAY_MS>, ConservativeCondition<MAX_ATTEMPTS>>> {
    RetryStrategy::with_policy(FixedDelay::default(), ConservativeCondition::new())
}

/// Create an aggressive retry strategy.
pub fn aggressive_retry<const MAX_ATTEMPTS: usize>()
-> ConfigResult<RetryStrategy<ExponentialBackoff<50, 15>, AggressiveCondition<MAX_ATTEMPTS>>> {
    RetryStrategy::with_policy(ExponentialBackoff::default(), AggressiveCondition::new())
}

/// Standalone retry function with automatic strategy inference
pub async fn retry<T, E, F, Fut, B, C>(
    strategy: &RetryStrategy<B, C>,
    operation: F,
) -> Result<(T, RetryStats), E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: fmt::Debug,
    B: BackoffPolicy,
    C: RetryCondition<E>,
{
    strategy.execute(operation).await
}

/// Simple retry function with default exponential backoff
pub async fn retry_with_backoff<T, E, F, Fut>(
    max_attempts: usize,
    base_delay: Duration,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: fmt::Debug,
{
    let mut attempts = 0;
    let mut current_delay = base_delay;

    loop {
        attempts += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) if attempts >= max_attempts => return Err(e),
            Err(_) => {
                tokio::time::sleep(current_delay).await;
                // Exponential backoff with cap
                current_delay = std::cmp::min(current_delay * 2, Duration::from_secs(60));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::Duration;

    #[test]
    fn test_const_backoff_validation() {
        // These should compile
        const _VALID_FIXED: FixedDelay<1000> = FixedDelay {
            _marker: PhantomData,
        };
        const _VALID_LINEAR: LinearBackoff<100, 5000> = LinearBackoff {
            _marker: PhantomData,
        };
        const _VALID_EXP: ExponentialBackoff<100, 20, 5000> = ExponentialBackoff {
            _marker: PhantomData,
        };

        // Runtime validation
        assert!(FixedDelay::<1000>::is_valid());
        assert!(LinearBackoff::<100, 5000>::is_valid());
        assert!(ExponentialBackoff::<100, 20, 5000>::is_valid());

        // Invalid cases
        assert!(!FixedDelay::<0>::is_valid());
        assert!(!LinearBackoff::<0, 1000>::is_valid());
        assert!(!ExponentialBackoff::<100, 5, 5000>::is_valid()); // Multiplier too low
    }

    #[test]
    fn test_retry_condition_with_typed_errors() {
        use crate::core::ResilienceError;

        let condition = ConservativeCondition::<3>::new();

        // Should retry on Timeout
        let timeout_err = ResilienceError::timeout(Duration::from_secs(1));
        assert!(condition.should_retry(&timeout_err, 0, Duration::ZERO));

        // Should NOT retry on InvalidConfig
        let config_err = ResilienceError::InvalidConfig {
            message: "bad".into(),
        };
        assert!(!condition.should_retry(&config_err, 0, Duration::ZERO));

        // Terminal errors should return true for is_terminal
        assert!(condition.is_terminal(&config_err));
    }

    #[test]
    fn test_backoff_calculations() {
        let fixed = FixedDelay::<1000>::default();
        assert_eq!(fixed.calculate_delay(0), Duration::from_millis(1000));
        assert_eq!(fixed.calculate_delay(5), Duration::from_millis(1000));

        let linear = LinearBackoff::<100, 5000>::default();
        assert_eq!(linear.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(linear.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(linear.calculate_delay(2), Duration::from_millis(300));

        let exp = ExponentialBackoff::<100, 20, 5000>::default();
        assert_eq!(exp.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(exp.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(exp.calculate_delay(2), Duration::from_millis(400));
    }

    #[test]
    fn test_jitter_policies() {
        let delay = Duration::from_millis(1000);

        assert_eq!(JitterPolicy::None.apply(delay, None), delay);

        let full_jitter = JitterPolicy::Full.apply(delay, None);
        assert!(full_jitter <= delay);

        let equal_jitter = JitterPolicy::Equal.apply(delay, None);
        assert!(equal_jitter >= Duration::from_millis(500));
        assert!(equal_jitter <= delay);

        let decorrelated =
            JitterPolicy::Decorrelated.apply(delay, Some(Duration::from_millis(800)));
        assert!(decorrelated >= delay); // Should be at least base delay
    }

    #[tokio::test]
    async fn test_retry_strategy_success() {
        use crate::core::ResilienceError;

        let strategy = RetryStrategy::with_policy(
            FixedDelay::<10>::default(),
            ConservativeCondition::<3>::new(),
        )
        .unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let (result, stats) = strategy
            .execute(|| async {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Ok::<_, ResilienceError>("success")
            })
            .await
            .unwrap();

        assert_eq!(result, "success");
        assert_eq!(stats.total_attempts, 1);
        assert!(stats.succeeded);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_retry_strategy_with_failures() {
        let strategy = RetryStrategy::with_policy(
            FixedDelay::<10>::default(),
            AggressiveCondition::<3>::new(),
        )
        .unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = strategy
            .execute(|| async {
                let count = counter_clone.fetch_add(1, Ordering::Relaxed);
                if count < 2 {
                    Err("temporary failure")
                } else {
                    Ok("success")
                }
            })
            .await;

        match result {
            Ok((value, stats)) => {
                assert_eq!(value, "success");
                assert_eq!(stats.total_attempts, 3);
                assert!(stats.succeeded);
                assert_eq!(counter.load(Ordering::Relaxed), 3);
            }
            Err(_) => panic!("Expected success after retries"),
        }
    }

    #[tokio::test]
    async fn test_retry_terminal_error() {
        use crate::core::ResilienceError;

        let strategy = RetryStrategy::with_policy(
            FixedDelay::<10>::default(),
            ConservativeCondition::<3>::new(),
        )
        .unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = strategy
            .execute(|| async {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Err::<(), _>(ResilienceError::InvalidConfig {
                    message: "NotFound".into(),
                }) // Terminal error
            })
            .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), 1); // Should not retry
    }

    #[tokio::test]
    async fn test_time_based_condition() {
        let strategy = RetryStrategy::with_policy(
            FixedDelay::<100>::default(),
            TimeBasedCondition::<200>::new(10), // Very short time limit
        )
        .unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let start = Instant::now();
        let result = strategy
            .execute(|| async {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                Err::<(), _>("always fails")
            })
            .await;

        let elapsed = start.elapsed();
        assert!(result.is_err());

        // Should have stopped due to time limit
        assert!(elapsed >= Duration::from_millis(200));
        assert!(elapsed < Duration::from_millis(1000)); // Shouldn't take too long
    }

    #[tokio::test]
    async fn test_helper_functions() {
        let _standard: Result<
            RetryStrategy<ExponentialBackoff<100, 20>, ConservativeCondition<3>>,
            ConfigError,
        > = exponential_retry::<3>();

        let _quick: Result<RetryStrategy<FixedDelay<50>, ConservativeCondition<2>>, ConfigError> =
            fixed_retry::<50, 2>();

        let _aggressive: Result<
            RetryStrategy<ExponentialBackoff<50, 15>, AggressiveCondition<5>>,
            ConfigError,
        > = aggressive_retry::<5>();

        assert!(_standard.is_ok());
        assert!(_quick.is_ok());
        assert!(_aggressive.is_ok());
    }

    #[tokio::test]
    async fn test_simple_retry_function() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result = retry_with_backoff(3, Duration::from_millis(10), || async {
            let count = counter_clone.fetch_add(1, Ordering::Relaxed);
            if count < 2 {
                Err("temporary")
            } else {
                Ok("success")
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(counter.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn test_custom_backoff() {
        let custom = CustomBackoff::from_millis(&[100, 200, 400], 1000);

        assert_eq!(custom.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(custom.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(custom.calculate_delay(2), Duration::from_millis(400));
        assert_eq!(custom.calculate_delay(10), Duration::from_millis(1000)); // Default
    }
}
