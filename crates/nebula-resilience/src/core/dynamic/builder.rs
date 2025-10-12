//! Type-safe builder API for dynamic configuration
//!
//! This module provides a type-safe, compile-time checked API for building
//! dynamic configurations, complementing the existing runtime string-based API.
//!
//! # Examples
//!
//! ```
//! use nebula_resilience::DynamicConfigBuilder;
//! use std::time::Duration;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DynamicConfigBuilder::new()
//!     .retry()
//!         .max_attempts(3)
//!         .base_delay(Duration::from_millis(100))
//!         .done()?
//!     .circuit_breaker()
//!         .failure_threshold(5)
//!         .reset_timeout(Duration::from_secs(30))
//!         .done()?
//!     .build();
//! # Ok(())
//! # }
//! ```

use crate::core::config::{ConfigError, ConfigResult};
use crate::core::dynamic::DynamicConfig;
use nebula_value::Value;
use std::time::Duration;

/// Type-safe builder for dynamic configuration
///
/// Provides compile-time type checking and IDE autocomplete support
/// for building dynamic configurations.
#[derive(Debug, Clone)]
pub struct DynamicConfigBuilder {
    inner: DynamicConfig,
}

impl DynamicConfigBuilder {
    /// Create a new dynamic configuration builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: DynamicConfig::new(),
        }
    }

    /// Start building retry configuration
    #[must_use]
    pub fn retry(self) -> RetryConfigBuilder {
        RetryConfigBuilder::new(self)
    }

    /// Start building circuit breaker configuration
    #[must_use]
    pub fn circuit_breaker(self) -> CircuitBreakerConfigBuilder {
        CircuitBreakerConfigBuilder::new(self)
    }

    /// Start building bulkhead configuration
    #[must_use]
    pub fn bulkhead(self) -> BulkheadConfigBuilder {
        BulkheadConfigBuilder::new(self)
    }

    /// Build the final dynamic configuration
    #[must_use]
    pub fn build(self) -> DynamicConfig {
        self.inner
    }
}

impl Default for DynamicConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for retry configuration with compile-time type safety
#[derive(Debug)]
pub struct RetryConfigBuilder {
    parent: DynamicConfigBuilder,
    max_attempts: Option<usize>,
    base_delay: Option<Duration>,
}

impl RetryConfigBuilder {
    fn new(parent: DynamicConfigBuilder) -> Self {
        Self {
            parent,
            max_attempts: None,
            base_delay: None,
        }
    }

    /// Set maximum retry attempts
    ///
    /// # Type Safety
    /// Only accepts `usize`, preventing runtime type errors.
    #[must_use]
    pub fn max_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = Some(attempts);
        self
    }

    /// Set base delay between retries
    ///
    /// # Type Safety
    /// Only accepts `Duration`, preventing runtime type errors.
    #[must_use]
    pub fn base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = Some(delay);
        self
    }

    /// Finish retry configuration and return to parent builder
    ///
    /// # Errors
    /// Returns error if required fields are not set.
    pub fn done(mut self) -> ConfigResult<DynamicConfigBuilder> {
        // Validate required fields at build time
        let max_attempts = self
            .max_attempts
            .ok_or_else(|| ConfigError::validation("retry.max_attempts is required"))?;

        let base_delay = self
            .base_delay
            .ok_or_else(|| ConfigError::validation("retry.base_delay is required"))?;

        // Additional validation
        if max_attempts == 0 {
            return Err(ConfigError::validation(
                "retry.max_attempts must be greater than 0",
            ));
        }

        if base_delay.is_zero() {
            return Err(ConfigError::validation(
                "retry.base_delay must be greater than 0",
            ));
        }

        // Set values in parent config
        self.parent.inner.set_value(
            "retry.max_attempts",
            Value::from(max_attempts as i64),
        )?;

        self.parent.inner.set_value(
            "retry.base_delay_ms",
            Value::from(base_delay.as_millis() as i64),
        )?;

        Ok(self.parent)
    }
}

/// Builder for circuit breaker configuration with compile-time type safety
#[derive(Debug)]
pub struct CircuitBreakerConfigBuilder {
    parent: DynamicConfigBuilder,
    failure_threshold: Option<usize>,
    reset_timeout: Option<Duration>,
    half_open_max_operations: Option<usize>,
}

impl CircuitBreakerConfigBuilder {
    fn new(parent: DynamicConfigBuilder) -> Self {
        Self {
            parent,
            failure_threshold: None,
            reset_timeout: None,
            half_open_max_operations: None,
        }
    }

    /// Set failure threshold before opening circuit
    #[must_use]
    pub fn failure_threshold(mut self, threshold: usize) -> Self {
        self.failure_threshold = Some(threshold);
        self
    }

    /// Set timeout before attempting to close circuit
    #[must_use]
    pub fn reset_timeout(mut self, timeout: Duration) -> Self {
        self.reset_timeout = Some(timeout);
        self
    }

    /// Set maximum operations allowed in half-open state
    #[must_use]
    pub fn half_open_max_operations(mut self, max_ops: usize) -> Self {
        self.half_open_max_operations = Some(max_ops);
        self
    }

    /// Finish circuit breaker configuration
    ///
    /// # Errors
    /// Returns error if required fields are not set.
    pub fn done(mut self) -> ConfigResult<DynamicConfigBuilder> {
        let failure_threshold = self
            .failure_threshold
            .ok_or_else(|| ConfigError::validation("circuit_breaker.failure_threshold is required"))?;

        let reset_timeout = self
            .reset_timeout
            .ok_or_else(|| ConfigError::validation("circuit_breaker.reset_timeout is required"))?;

        // Validation
        if failure_threshold == 0 {
            return Err(ConfigError::validation(
                "circuit_breaker.failure_threshold must be greater than 0",
            ));
        }

        if reset_timeout.is_zero() {
            return Err(ConfigError::validation(
                "circuit_breaker.reset_timeout must be greater than 0",
            ));
        }

        // Set required values
        self.parent.inner.set_value(
            "circuit_breaker.failure_threshold",
            Value::from(failure_threshold as i64),
        )?;

        self.parent.inner.set_value(
            "circuit_breaker.reset_timeout_ms",
            Value::from(reset_timeout.as_millis() as i64),
        )?;

        // Set optional values
        if let Some(max_ops) = self.half_open_max_operations {
            self.parent.inner.set_value(
                "circuit_breaker.half_open_max_operations",
                Value::from(max_ops as i64),
            )?;
        }

        Ok(self.parent)
    }
}

/// Builder for bulkhead configuration with compile-time type safety
#[derive(Debug)]
pub struct BulkheadConfigBuilder {
    parent: DynamicConfigBuilder,
    max_concurrency: Option<usize>,
    queue_size: Option<usize>,
    timeout: Option<Duration>,
}

impl BulkheadConfigBuilder {
    fn new(parent: DynamicConfigBuilder) -> Self {
        Self {
            parent,
            max_concurrency: None,
            queue_size: None,
            timeout: None,
        }
    }

    /// Set maximum concurrent operations
    #[must_use]
    pub fn max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = Some(max);
        self
    }

    /// Set queue size for waiting operations
    #[must_use]
    pub fn queue_size(mut self, size: usize) -> Self {
        self.queue_size = Some(size);
        self
    }

    /// Set timeout for acquiring bulkhead permit
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Finish bulkhead configuration
    ///
    /// # Errors
    /// Returns error if required fields are not set.
    pub fn done(mut self) -> ConfigResult<DynamicConfigBuilder> {
        let max_concurrency = self
            .max_concurrency
            .ok_or_else(|| ConfigError::validation("bulkhead.max_concurrency is required"))?;

        let queue_size = self
            .queue_size
            .ok_or_else(|| ConfigError::validation("bulkhead.queue_size is required"))?;

        // Validation
        if max_concurrency == 0 {
            return Err(ConfigError::validation(
                "bulkhead.max_concurrency must be greater than 0",
            ));
        }

        // Set required values
        self.parent.inner.set_value(
            "bulkhead.max_concurrency",
            Value::from(max_concurrency as i64),
        )?;

        self.parent.inner.set_value(
            "bulkhead.queue_size",
            Value::from(queue_size as i64),
        )?;

        // Set optional values
        if let Some(timeout) = self.timeout {
            self.parent.inner.set_value(
                "bulkhead.timeout_ms",
                Value::from(timeout.as_millis() as i64),
            )?;
        }

        Ok(self.parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_builder() {
        let config = DynamicConfigBuilder::new()
            .retry()
            .max_attempts(3)
            .base_delay(Duration::from_millis(100))
            .done()
            .unwrap()
            .build();

        let max_attempts = config.get_value("retry.max_attempts").unwrap();
        assert_eq!(max_attempts, Value::from(3i64));

        let base_delay = config.get_value("retry.base_delay_ms").unwrap();
        assert_eq!(base_delay, Value::from(100i64));
    }

    #[test]
    fn test_circuit_breaker_builder() {
        let config = DynamicConfigBuilder::new()
            .circuit_breaker()
            .failure_threshold(5)
            .reset_timeout(Duration::from_secs(30))
            .half_open_max_operations(2)
            .done()
            .unwrap()
            .build();

        let threshold = config
            .get_value("circuit_breaker.failure_threshold")
            .unwrap();
        assert_eq!(threshold, Value::from(5i64));

        let timeout = config
            .get_value("circuit_breaker.reset_timeout_ms")
            .unwrap();
        assert_eq!(timeout, Value::from(30000i64));
    }

    #[test]
    fn test_bulkhead_builder() {
        let config = DynamicConfigBuilder::new()
            .bulkhead()
            .max_concurrency(10)
            .queue_size(20)
            .timeout(Duration::from_secs(5))
            .done()
            .unwrap()
            .build();

        let max_concurrency = config.get_value("bulkhead.max_concurrency").unwrap();
        assert_eq!(max_concurrency, Value::from(10i64));

        let queue_size = config.get_value("bulkhead.queue_size").unwrap();
        assert_eq!(queue_size, Value::from(20i64));
    }

    #[test]
    fn test_chained_builders() {
        let config = DynamicConfigBuilder::new()
            .retry()
            .max_attempts(3)
            .base_delay(Duration::from_millis(100))
            .done()
            .unwrap()
            .circuit_breaker()
            .failure_threshold(5)
            .reset_timeout(Duration::from_secs(30))
            .done()
            .unwrap()
            .bulkhead()
            .max_concurrency(10)
            .queue_size(20)
            .done()
            .unwrap()
            .build();

        // Verify all configs are set
        assert!(config.get_value("retry.max_attempts").is_ok());
        assert!(config
            .get_value("circuit_breaker.failure_threshold")
            .is_ok());
        assert!(config.get_value("bulkhead.max_concurrency").is_ok());
    }

    #[test]
    fn test_validation_zero_attempts() {
        let result = DynamicConfigBuilder::new()
            .retry()
            .max_attempts(0)
            .base_delay(Duration::from_millis(100))
            .done();

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be greater than 0"));
    }

    #[test]
    fn test_validation_missing_required_field() {
        let result = DynamicConfigBuilder::new()
            .retry()
            .max_attempts(3)
            // Missing base_delay
            .done();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("required"));
    }
}
