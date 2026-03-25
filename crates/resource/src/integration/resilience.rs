//! Resilience configuration for resource acquisition.
//!
//! [`AcquireResilience`] bundles timeout, retry, and circuit-breaker
//! settings into a single config object. Three presets cover the most
//! common use cases.
//!
//! # Examples
//!
//! ```
//! use nebula_resource::integration::AcquireResilience;
//!
//! let config = AcquireResilience::standard();
//! assert!(config.timeout.is_some());
//! ```

use std::time::Duration;

/// Resilience configuration applied when acquiring a resource.
///
/// Combines optional timeout, retry, and circuit-breaker settings.
/// Use one of the preset constructors ([`standard`](Self::standard),
/// [`fast`](Self::fast), [`slow`](Self::slow)) or build manually.
#[derive(Debug, Clone)]
pub struct AcquireResilience {
    /// Overall acquire timeout (wall-clock, including retries).
    pub timeout: Option<Duration>,
    /// Retry policy for transient acquisition failures.
    pub retry: Option<AcquireRetryConfig>,
    /// Circuit-breaker preset for the underlying backend.
    pub circuit_breaker: Option<AcquireCircuitBreakerPreset>,
}

/// Retry policy for resource acquisition.
#[derive(Debug, Clone)]
pub struct AcquireRetryConfig {
    /// Maximum number of retry attempts (excluding the initial try).
    pub max_attempts: u32,
    /// Initial backoff duration before the first retry.
    pub initial_backoff: Duration,
    /// Maximum backoff duration (caps exponential growth).
    pub max_backoff: Duration,
}

/// Circuit-breaker presets for resource acquisition.
///
/// Each preset defines a failure threshold and reset timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireCircuitBreakerPreset {
    /// 5 failures, 30 s reset window.
    Standard,
    /// 3 failures, 10 s reset window — for latency-sensitive paths.
    Fast,
    /// 10 failures, 60 s reset window — for noisy backends.
    Slow,
}

impl AcquireResilience {
    /// Balanced defaults: 30 s timeout, 3 retries, standard breaker.
    pub fn standard() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 3,
                initial_backoff: Duration::from_millis(200),
                max_backoff: Duration::from_secs(5),
            }),
            circuit_breaker: Some(AcquireCircuitBreakerPreset::Standard),
        }
    }

    /// Low-latency: 10 s timeout, 2 retries, fast breaker.
    pub fn fast() -> Self {
        Self {
            timeout: Some(Duration::from_secs(10)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(100),
                max_backoff: Duration::from_secs(2),
            }),
            circuit_breaker: Some(AcquireCircuitBreakerPreset::Fast),
        }
    }

    /// Tolerant: 60 s timeout, 5 retries, slow breaker.
    pub fn slow() -> Self {
        Self {
            timeout: Some(Duration::from_secs(60)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 5,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(15),
            }),
            circuit_breaker: Some(AcquireCircuitBreakerPreset::Slow),
        }
    }

    /// No resilience — bare acquire with no timeout, retries, or breaker.
    pub fn none() -> Self {
        Self {
            timeout: None,
            retry: None,
            circuit_breaker: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_preset_has_all_fields() {
        let c = AcquireResilience::standard();
        assert!(c.timeout.is_some());
        assert!(c.retry.is_some());
        assert_eq!(
            c.circuit_breaker,
            Some(AcquireCircuitBreakerPreset::Standard)
        );
    }

    #[test]
    fn fast_preset_is_lower_latency() {
        let c = AcquireResilience::fast();
        assert!(c.timeout.unwrap() < Duration::from_secs(30));
        assert_eq!(c.retry.as_ref().unwrap().max_attempts, 2);
        assert_eq!(c.circuit_breaker, Some(AcquireCircuitBreakerPreset::Fast));
    }

    #[test]
    fn slow_preset_is_more_tolerant() {
        let c = AcquireResilience::slow();
        assert!(c.timeout.unwrap() > Duration::from_secs(30));
        assert_eq!(c.retry.as_ref().unwrap().max_attempts, 5);
        assert_eq!(c.circuit_breaker, Some(AcquireCircuitBreakerPreset::Slow));
    }

    #[test]
    fn none_preset_has_nothing() {
        let c = AcquireResilience::none();
        assert!(c.timeout.is_none());
        assert!(c.retry.is_none());
        assert!(c.circuit_breaker.is_none());
    }
}
