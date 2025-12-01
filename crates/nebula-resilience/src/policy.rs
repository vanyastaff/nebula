//! Modern resilience policies for service configuration
//!
//! This module provides type-safe resilience policies using the new
//! generic retry strategy system.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::{
    core::config::{ConfigError, ConfigResult, ResilienceConfig},
    patterns::{bulkhead::BulkheadConfig, circuit_breaker::CircuitBreakerConfig},
};

/// Retry configuration for policies (simplified, serializable version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicyConfig {
    /// Maximum retry attempts
    pub max_attempts: usize,
    /// Base delay in milliseconds
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds
    pub max_delay_ms: u64,
    /// Backoff multiplier (x10 to avoid floats in serialization)
    pub multiplier_x10: u64,
    /// Whether to use jitter
    pub use_jitter: bool,
}

impl Default for RetryPolicyConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            multiplier_x10: 20, // 2.0x multiplier
            use_jitter: true,
        }
    }
}

impl RetryPolicyConfig {
    /// Create exponential backoff configuration
    pub fn exponential(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay_ms: base_delay.as_millis() as u64,
            max_delay_ms: 30_000,
            multiplier_x10: 20,
            use_jitter: true,
        }
    }

    /// Create fixed delay configuration
    pub fn fixed(max_attempts: usize, delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay_ms: delay.as_millis() as u64,
            max_delay_ms: delay.as_millis() as u64,
            multiplier_x10: 10, // 1.0x (no growth)
            use_jitter: false,
        }
    }

    /// Calculate delay for a given attempt
    pub fn delay_for_attempt(&self, attempt: usize) -> Option<Duration> {
        if attempt >= self.max_attempts {
            return None;
        }

        let multiplier = self.multiplier_x10 as f64 / 10.0;
        let delay = (self.base_delay_ms as f64) * multiplier.powi(attempt as i32);
        let capped = (delay as u64).min(self.max_delay_ms);

        Some(Duration::from_millis(capped))
    }

    /// Validate the configuration
    pub fn validate(&self) -> ConfigResult<()> {
        if self.max_attempts == 0 {
            return Err(ConfigError::validation("max_attempts must be > 0"));
        }
        if self.base_delay_ms == 0 {
            return Err(ConfigError::validation("base_delay_ms must be > 0"));
        }
        if self.max_delay_ms < self.base_delay_ms {
            return Err(ConfigError::validation(
                "max_delay_ms must be >= base_delay_ms",
            ));
        }
        Ok(())
    }
}

/// Modern resilience policy with type-safe configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResiliencePolicy {
    /// Timeout configuration
    pub timeout: Option<Duration>,
    /// Retry strategy configuration
    pub retry: Option<RetryPolicyConfig>,
    /// Circuit breaker configuration
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    /// Bulkhead configuration
    pub bulkhead: Option<BulkheadConfig>,
    /// Policy metadata
    pub metadata: PolicyMetadata,
}

/// Metadata for resilience policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMetadata {
    /// Policy name
    pub name: String,
    /// Policy description
    pub description: Option<String>,
    /// Policy version
    pub version: String,
    /// Policy tags for categorization
    pub tags: Vec<String>,
    /// Policy priority (higher number = higher priority)
    pub priority: u32,
}

impl Default for PolicyMetadata {
    fn default() -> Self {
        Self {
            name: "default-policy".to_string(),
            description: None,
            version: "1.0.0".to_string(),
            tags: Vec::new(),
            priority: 100,
        }
    }
}

impl Default for ResiliencePolicy {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            retry: Some(RetryPolicyConfig::default()),
            circuit_breaker: None,
            bulkhead: None,
            metadata: PolicyMetadata::default(),
        }
    }
}

impl ResiliencePolicy {
    /// Create a new resilience policy with name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: PolicyMetadata {
                name: name.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a basic policy with timeout and retry
    #[must_use]
    pub fn basic(timeout: Duration, retry_attempts: usize) -> Self {
        Self {
            timeout: Some(timeout),
            retry: Some(RetryPolicyConfig::exponential(
                retry_attempts,
                Duration::from_millis(100),
            )),
            circuit_breaker: None,
            bulkhead: None,
            metadata: PolicyMetadata {
                name: "basic-policy".to_string(),
                description: Some("Basic resilience policy with timeout and retry".to_string()),
                ..Default::default()
            },
        }
    }

    /// Create a robust policy with all resilience patterns
    #[must_use]
    pub fn robust(
        timeout: Duration,
        retry_attempts: usize,
        circuit_breaker: CircuitBreakerConfig,
        bulkhead: BulkheadConfig,
    ) -> Self {
        Self {
            timeout: Some(timeout),
            retry: Some(RetryPolicyConfig::exponential(
                retry_attempts,
                Duration::from_millis(100),
            )),
            circuit_breaker: Some(circuit_breaker),
            bulkhead: Some(bulkhead),
            metadata: PolicyMetadata {
                name: "robust-policy".to_string(),
                description: Some("Comprehensive resilience policy with all patterns".to_string()),
                tags: vec!["production".to_string(), "high-availability".to_string()],
                priority: 200,
                ..Default::default()
            },
        }
    }

    /// Create a policy optimized for microservices
    #[must_use]
    pub fn microservice() -> Self {
        Self {
            timeout: Some(Duration::from_secs(10)),
            retry: Some(RetryPolicyConfig::exponential(3, Duration::from_millis(50))),
            circuit_breaker: Some(CircuitBreakerConfig::default()),
            bulkhead: Some(BulkheadConfig::default()),
            metadata: PolicyMetadata {
                name: "microservice-policy".to_string(),
                description: Some("Optimized policy for microservice communication".to_string()),
                tags: vec!["microservice".to_string(), "default".to_string()],
                priority: 150,
                ..Default::default()
            },
        }
    }

    /// Set metadata
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metadata(mut self, metadata: PolicyMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set name
    #[must_use = "builder methods must be chained or built"]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.metadata.name = name.into();
        self
    }

    /// Set description
    #[must_use = "builder methods must be chained or built"]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }

    /// Add tag
    #[must_use = "builder methods must be chained or built"]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }

    /// Set priority
    #[must_use = "builder methods must be chained or built"]
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.metadata.priority = priority;
        self
    }

    /// Set timeout
    #[must_use = "builder methods must be chained or built"]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set retry strategy
    #[must_use = "builder methods must be chained or built"]
    pub fn with_retry(mut self, config: RetryPolicyConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Set circuit breaker
    #[must_use = "builder methods must be chained or built"]
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Some(config);
        self
    }

    /// Set bulkhead
    #[must_use = "builder methods must be chained or built"]
    pub fn with_bulkhead(mut self, config: BulkheadConfig) -> Self {
        self.bulkhead = Some(config);
        self
    }

    /// Remove timeout
    #[must_use = "builder methods must be chained or built"]
    pub fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Remove retry
    #[must_use = "builder methods must be chained or built"]
    pub fn without_retry(mut self) -> Self {
        self.retry = None;
        self
    }

    /// Remove circuit breaker
    #[must_use = "builder methods must be chained or built"]
    pub fn without_circuit_breaker(mut self) -> Self {
        self.circuit_breaker = None;
        self
    }

    /// Remove bulkhead
    #[must_use = "builder methods must be chained or built"]
    pub fn without_bulkhead(mut self) -> Self {
        self.bulkhead = None;
        self
    }

    /// Check if policy has any resilience patterns enabled
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.timeout.is_some()
            || self.retry.is_some()
            || self.circuit_breaker.is_some()
            || self.bulkhead.is_some()
    }

    /// Get estimated maximum execution time including retries
    #[must_use]
    pub fn max_execution_time(&self) -> Option<Duration> {
        let base_timeout = self.timeout.unwrap_or(Duration::from_secs(60));

        if let Some(retry) = &self.retry {
            let retry_time: Duration = (0..retry.max_attempts)
                .filter_map(|attempt| retry.delay_for_attempt(attempt))
                .sum();

            Some(base_timeout * retry.max_attempts as u32 + retry_time)
        } else {
            Some(base_timeout)
        }
    }

    /// Merge with another policy (other takes precedence)
    #[must_use = "builder methods must be chained or built"]
    pub fn merge(mut self, other: Self) -> Self {
        if other.timeout.is_some() {
            self.timeout = other.timeout;
        }
        if other.retry.is_some() {
            self.retry = other.retry;
        }
        if other.circuit_breaker.is_some() {
            self.circuit_breaker = other.circuit_breaker;
        }
        if other.bulkhead.is_some() {
            self.bulkhead = other.bulkhead;
        }

        // Merge metadata
        if !other.metadata.name.is_empty() && other.metadata.name != "default-policy" {
            self.metadata.name = other.metadata.name;
        }
        if other.metadata.description.is_some() {
            self.metadata.description = other.metadata.description;
        }
        if !other.metadata.tags.is_empty() {
            self.metadata.tags.extend(other.metadata.tags);
            self.metadata.tags.dedup();
        }
        if other.metadata.priority != 100 {
            self.metadata.priority = other.metadata.priority;
        }

        self
    }
}

impl ResilienceConfig for ResiliencePolicy {
    fn validate(&self) -> ConfigResult<()> {
        // Validate timeout
        if let Some(timeout) = self.timeout {
            if timeout.is_zero() {
                return Err(ConfigError::validation("Timeout cannot be zero"));
            }
            if timeout > Duration::from_secs(3600) {
                return Err(ConfigError::validation("Timeout cannot exceed 1 hour"));
            }
        }

        // Validate retry strategy
        if let Some(retry) = &self.retry {
            retry.validate()?;
        }

        // Validate circuit breaker
        if let Some(cb) = &self.circuit_breaker {
            cb.validate()?;
        }

        // Validate bulkhead
        if let Some(bulkhead) = &self.bulkhead {
            bulkhead.validate()?;
        }

        // Validate metadata
        if self.metadata.name.is_empty() {
            return Err(ConfigError::validation("Policy name cannot be empty"));
        }

        // Check for conflicting configurations
        if let (Some(_timeout), Some(_retry)) = (self.timeout, &self.retry)
            && let Some(max_exec_time) = self.max_execution_time()
            && max_exec_time > Duration::from_secs(600)
        {
            return Err(ConfigError::validation(
                "Combined timeout and retry configuration would exceed 10 minutes",
            ));
        }

        Ok(())
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        *self = self.clone().merge(other);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_creation() {
        let policy = ResiliencePolicy::basic(Duration::from_secs(10), 3);
        assert!(policy.timeout.is_some());
        assert!(policy.retry.is_some());
        assert!(policy.is_enabled());
    }

    #[test]
    fn test_policy_validation() {
        let policy = ResiliencePolicy::basic(Duration::from_secs(10), 3);
        assert!(policy.validate().is_ok());

        let invalid_policy = ResiliencePolicy::new("").with_timeout(Duration::ZERO);
        assert!(invalid_policy.validate().is_err());
    }

    #[test]
    fn test_policy_merge() {
        let base = ResiliencePolicy::basic(Duration::from_secs(10), 3);
        let override_policy =
            ResiliencePolicy::new("override").with_timeout(Duration::from_secs(20));

        let merged = base.merge(override_policy);
        assert_eq!(merged.timeout, Some(Duration::from_secs(20)));
        assert!(merged.retry.is_some());
    }

    #[test]
    fn test_max_execution_time() {
        let policy = ResiliencePolicy::basic(Duration::from_secs(10), 3);
        let max_time = policy.max_execution_time();
        assert!(max_time.is_some());
        assert!(max_time.unwrap() > Duration::from_secs(10));
    }

    #[test]
    fn test_retry_policy_config() {
        let config = RetryPolicyConfig::exponential(3, Duration::from_millis(100));

        assert_eq!(
            config.delay_for_attempt(0),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            config.delay_for_attempt(1),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            config.delay_for_attempt(2),
            Some(Duration::from_millis(400))
        );
        assert_eq!(config.delay_for_attempt(3), None); // exceeds max_attempts
    }

    #[test]
    fn test_fixed_retry_config() {
        let config = RetryPolicyConfig::fixed(5, Duration::from_millis(500));

        assert_eq!(
            config.delay_for_attempt(0),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            config.delay_for_attempt(1),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            config.delay_for_attempt(4),
            Some(Duration::from_millis(500))
        );
        assert_eq!(config.delay_for_attempt(5), None);
    }
}
