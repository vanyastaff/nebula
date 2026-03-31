//! Resilience configuration for resource acquisition.
//!
//! [`AcquireResilience`] bundles timeout and retry settings into a
//! single config object. Three presets cover the most common use cases.
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

use nebula_resilience::retry::{BackoffConfig, RetryConfig};

/// Resilience configuration applied when acquiring a resource.
///
/// Combines optional timeout and retry settings.
/// Use one of the preset constructors ([`standard`](Self::standard),
/// [`fast`](Self::fast), [`slow`](Self::slow)) or build manually.
///
/// Internally converts to [`nebula_resilience::RetryConfig`] via
/// [`to_retry_config`](Self::to_retry_config).
#[derive(Debug, Clone)]
pub struct AcquireResilience {
    /// Overall acquire timeout (wall-clock, including retries).
    pub timeout: Option<Duration>,
    /// Retry policy for transient acquisition failures.
    pub retry: Option<AcquireRetryConfig>,
}

/// Retry policy for resource acquisition.
#[derive(Debug, Clone)]
pub struct AcquireRetryConfig {
    /// Maximum total number of attempts (including the initial try).
    ///
    /// For example, `max_attempts: 3` means 1 initial try + 2 retries.
    pub max_attempts: u32,
    /// Initial backoff duration before the first retry.
    pub initial_backoff: Duration,
    /// Maximum backoff duration (caps exponential growth).
    pub max_backoff: Duration,
}

impl AcquireResilience {
    /// Balanced defaults: 30 s timeout, 3 retries.
    pub fn standard() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 3,
                initial_backoff: Duration::from_millis(200),
                max_backoff: Duration::from_secs(5),
            }),
        }
    }

    /// Low-latency: 10 s timeout, 2 retries.
    pub fn fast() -> Self {
        Self {
            timeout: Some(Duration::from_secs(10)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(100),
                max_backoff: Duration::from_secs(2),
            }),
        }
    }

    /// Tolerant: 60 s timeout, 5 retries.
    pub fn slow() -> Self {
        Self {
            timeout: Some(Duration::from_secs(60)),
            retry: Some(AcquireRetryConfig {
                max_attempts: 5,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(15),
            }),
        }
    }

    /// No resilience — bare acquire with no timeout or retries.
    pub fn none() -> Self {
        Self {
            timeout: None,
            retry: None,
        }
    }

    /// Convert to a [`RetryConfig`] for use with [`nebula_resilience::retry_with`].
    ///
    /// Maps exponential backoff (2× multiplier) and optional wall-clock
    /// timeout budget. When no retry config is set, returns a single-attempt
    /// config with optional timeout budget.
    pub(crate) fn to_retry_config<E: 'static>(&self) -> RetryConfig<E> {
        let max_attempts = self.retry.as_ref().map_or(1, |r| r.max_attempts);
        let cfg = RetryConfig::new(max_attempts).expect("max_attempts validated at construction");

        let cfg = if let Some(ref retry) = self.retry {
            cfg.backoff(BackoffConfig::Exponential {
                base: retry.initial_backoff,
                multiplier: 2.0,
                max: retry.max_backoff,
            })
        } else {
            cfg
        };

        if let Some(timeout) = self.timeout {
            cfg.total_budget(timeout)
        } else {
            cfg
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
    }

    #[test]
    fn fast_preset_is_lower_latency() {
        let c = AcquireResilience::fast();
        assert!(c.timeout.unwrap() < Duration::from_secs(30));
        assert_eq!(c.retry.as_ref().unwrap().max_attempts, 2);
    }

    #[test]
    fn slow_preset_is_more_tolerant() {
        let c = AcquireResilience::slow();
        assert!(c.timeout.unwrap() > Duration::from_secs(30));
        assert_eq!(c.retry.as_ref().unwrap().max_attempts, 5);
    }

    #[test]
    fn none_preset_has_nothing() {
        let c = AcquireResilience::none();
        assert!(c.timeout.is_none());
        assert!(c.retry.is_none());
    }
}
