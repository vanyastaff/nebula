//! Type-safe newtypes and extension traits for resilience patterns
//!
//! This module provides:
//! - Newtype wrappers for compile-time type safety
//! - Extension traits for ergonomic APIs
//! - Const constructors for compile-time configuration
//!
//! # Newtype Pattern
//!
//! Newtypes prevent mixing up semantically different values of the same type:
//!
//! ```rust
//! use nebula_resilience::core::types::{FailureThreshold, MaxConcurrency};
//!
//! // These are different types even though both wrap usize
//! let failures = FailureThreshold::new(5);
//! let concurrency = MaxConcurrency::new(10);
//!
//! // Cannot accidentally mix them up!
//! // fn configure(threshold: FailureThreshold, concurrency: MaxConcurrency)
//! ```

use std::time::Duration;

// =============================================================================
// FAILURE THRESHOLD
// =============================================================================

/// Type-safe wrapper for failure threshold count.
///
/// Prevents accidentally mixing up failure thresholds with other numeric values.
///
/// # Example
///
/// ```rust
/// use nebula_resilience::core::types::FailureThreshold;
///
/// let threshold = FailureThreshold::new(5);
/// assert_eq!(threshold.get(), 5);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use = "FailureThreshold should be used in configuration"]
pub struct FailureThreshold(usize);

impl FailureThreshold {
    /// Default failure threshold (5 failures).
    pub const DEFAULT: Self = Self(5);

    /// Maximum allowed threshold (10,000).
    pub const MAX: Self = Self(10_000);

    /// Creates a new failure threshold.
    ///
    /// # Const
    ///
    /// This function is const, allowing compile-time configuration:
    ///
    /// ```rust
    /// use nebula_resilience::core::types::FailureThreshold;
    ///
    /// const MY_THRESHOLD: FailureThreshold = FailureThreshold::new(3);
    /// ```
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Returns the threshold value.
    #[inline]
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }

    /// Checks if the threshold is valid (> 0 and <= MAX).
    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 > 0 && self.0 <= Self::MAX.0
    }
}

impl Default for FailureThreshold {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<usize> for FailureThreshold {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl From<FailureThreshold> for usize {
    fn from(threshold: FailureThreshold) -> Self {
        threshold.0
    }
}

// =============================================================================
// MAX CONCURRENCY
// =============================================================================

/// Type-safe wrapper for maximum concurrency limit.
///
/// Prevents accidentally mixing up concurrency limits with other numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use = "MaxConcurrency should be used in configuration"]
pub struct MaxConcurrency(usize);

impl MaxConcurrency {
    /// Default concurrency limit (10).
    pub const DEFAULT: Self = Self(10);

    /// Minimum concurrency (1).
    pub const MIN: Self = Self(1);

    /// Creates a new concurrency limit.
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Returns the concurrency limit value.
    #[inline]
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }

    /// Checks if the limit is valid (> 0).
    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 > 0
    }
}

impl Default for MaxConcurrency {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<usize> for MaxConcurrency {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl From<MaxConcurrency> for usize {
    fn from(limit: MaxConcurrency) -> Self {
        limit.0
    }
}

// =============================================================================
// RETRY COUNT
// =============================================================================

/// Type-safe wrapper for retry attempt count.
///
/// Prevents accidentally mixing up retry counts with other numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use = "RetryCount should be used in configuration"]
pub struct RetryCount(usize);

impl RetryCount {
    /// No retries.
    pub const NONE: Self = Self(0);

    /// Default retry count (3).
    pub const DEFAULT: Self = Self(3);

    /// Maximum reasonable retry count (10).
    pub const MAX_REASONABLE: Self = Self(10);

    /// Creates a new retry count.
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Returns the retry count value.
    #[inline]
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }

    /// Checks if any retries are configured.
    #[inline]
    #[must_use]
    pub const fn has_retries(self) -> bool {
        self.0 > 0
    }
}

impl Default for RetryCount {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<usize> for RetryCount {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl From<RetryCount> for usize {
    fn from(count: RetryCount) -> Self {
        count.0
    }
}

// =============================================================================
// TIMEOUT DURATION
// =============================================================================

/// Type-safe wrapper for timeout duration.
///
/// Provides const constructors for common timeout values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use = "Timeout should be used in configuration"]
pub struct Timeout(Duration);

impl Timeout {
    /// No timeout (infinite wait).
    pub const NONE: Self = Self(Duration::MAX);

    /// Default timeout (30 seconds).
    pub const DEFAULT: Self = Self::from_secs(30);

    /// Short timeout (5 seconds).
    pub const SHORT: Self = Self::from_secs(5);

    /// Long timeout (60 seconds).
    pub const LONG: Self = Self::from_secs(60);

    /// Creates a timeout from seconds (const).
    #[inline]
    pub const fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }

    /// Creates a timeout from milliseconds (const).
    #[inline]
    pub const fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    /// Returns the duration.
    #[inline]
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.0
    }

    /// Checks if this represents no timeout.
    #[inline]
    #[must_use]
    pub const fn is_infinite(self) -> bool {
        self.0.as_secs() == Duration::MAX.as_secs()
    }
}

impl Default for Timeout {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<Duration> for Timeout {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl From<Timeout> for Duration {
    fn from(timeout: Timeout) -> Self {
        timeout.0
    }
}

// =============================================================================
// RATE LIMIT
// =============================================================================

/// Type-safe wrapper for rate limit (operations per second).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use = "RateLimit should be used in configuration"]
pub struct RateLimit(f64);

impl RateLimit {
    /// Default rate limit (100 ops/sec).
    pub const DEFAULT: Self = Self(100.0);

    /// Creates a new rate limit.
    #[inline]
    pub const fn new(ops_per_sec: f64) -> Self {
        Self(ops_per_sec)
    }

    /// Creates rate limit from operations per minute.
    #[inline]
    pub const fn per_minute(ops: f64) -> Self {
        Self(ops / 60.0)
    }

    /// Returns the rate limit in operations per second.
    #[inline]
    #[must_use]
    pub const fn ops_per_sec(self) -> f64 {
        self.0
    }

    /// Checks if the rate limit is valid (> 0).
    #[inline]
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.0 > 0.0 && self.0.is_finite()
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<f64> for RateLimit {
    fn from(rate: f64) -> Self {
        Self(rate)
    }
}

impl From<RateLimit> for f64 {
    fn from(limit: RateLimit) -> Self {
        limit.0
    }
}

// =============================================================================
// EXTENSION TRAITS
// =============================================================================

/// Extension trait for `Duration` with resilience-specific helpers.
pub trait DurationExt {
    /// Converts to a `Timeout` type.
    fn as_timeout(self) -> Timeout;

    /// Creates a jittered duration (adds randomness for retry backoff).
    fn with_jitter(self, factor: f64) -> Duration;

    /// Multiplies duration by a factor (for exponential backoff).
    fn multiply(self, factor: f64) -> Duration;
}

impl DurationExt for Duration {
    #[inline]
    fn as_timeout(self) -> Timeout {
        Timeout::from(self)
    }

    fn with_jitter(self, factor: f64) -> Duration {
        let random = fastrand::f64();
        let jitter_amount = self.as_secs_f64() * factor * random;
        Duration::from_secs_f64(self.as_secs_f64() + jitter_amount)
    }

    #[inline]
    fn multiply(self, factor: f64) -> Duration {
        Duration::from_secs_f64(self.as_secs_f64() * factor)
    }
}

/// Extension trait for `Result` with resilience helpers.
pub trait ResilienceResultExt<T, E> {
    /// Returns `true` if the error is retryable.
    fn is_retryable(&self) -> bool
    where
        E: crate::core::traits::Retryable;

    /// Converts error to a timeout error if it matches.
    fn or_timeout(self, duration: Duration) -> Result<T, crate::ResilienceError>
    where
        E: Into<crate::ResilienceError>;
}

impl<T, E> ResilienceResultExt<T, E> for Result<T, E> {
    fn is_retryable(&self) -> bool
    where
        E: crate::core::traits::Retryable,
    {
        match self {
            Ok(_) => false,
            Err(e) => e.is_retryable(),
        }
    }

    fn or_timeout(self, duration: Duration) -> Result<T, crate::ResilienceError>
    where
        E: Into<crate::ResilienceError>,
    {
        self.map_err(|e| {
            let err: crate::ResilienceError = e.into();
            if matches!(err, crate::ResilienceError::Timeout { .. }) {
                crate::ResilienceError::Timeout {
                    duration,
                    context: Some("Operation timed out".to_string()),
                }
            } else {
                err
            }
        })
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure_threshold_const() {
        const THRESHOLD: FailureThreshold = FailureThreshold::new(5);
        assert_eq!(THRESHOLD.get(), 5);
        assert!(THRESHOLD.is_valid());
    }

    #[test]
    fn test_failure_threshold_validation() {
        assert!(!FailureThreshold::new(0).is_valid());
        assert!(FailureThreshold::new(1).is_valid());
        assert!(FailureThreshold::new(10_000).is_valid());
        assert!(!FailureThreshold::new(10_001).is_valid());
    }

    #[test]
    fn test_max_concurrency() {
        const CONCURRENCY: MaxConcurrency = MaxConcurrency::new(20);
        assert_eq!(CONCURRENCY.get(), 20);
        assert!(CONCURRENCY.is_valid());
    }

    #[test]
    fn test_retry_count() {
        assert!(!RetryCount::NONE.has_retries());
        assert!(RetryCount::DEFAULT.has_retries());
        assert_eq!(RetryCount::DEFAULT.get(), 3);
    }

    #[test]
    fn test_timeout_const() {
        const TIMEOUT: Timeout = Timeout::from_secs(10);
        assert_eq!(TIMEOUT.duration(), Duration::from_secs(10));
        assert!(!TIMEOUT.is_infinite());
        assert!(Timeout::NONE.is_infinite());
    }

    #[test]
    fn test_rate_limit() {
        let limit = RateLimit::per_minute(60.0);
        assert!((limit.ops_per_sec() - 1.0).abs() < 0.001);
        assert!(limit.is_valid());
    }

    #[test]
    fn test_duration_ext_multiply() {
        let base = Duration::from_secs(1);
        let doubled = base.multiply(2.0);
        assert_eq!(doubled, Duration::from_secs(2));
    }

    #[test]
    fn test_duration_ext_jitter() {
        let base = Duration::from_secs(1);
        let jittered = base.with_jitter(0.1);
        // Should be between 1.0 and 1.1 seconds
        assert!(jittered >= base);
        assert!(jittered <= Duration::from_secs_f64(1.1));
    }
}
