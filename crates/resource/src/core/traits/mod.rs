//! Core traits for resource management

use async_trait::async_trait;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{context::ResourceContext, error::ResourceResult};

/// Health status for resource health checks
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HealthStatus {
    /// The health state
    pub state: HealthState,
    /// Latency of the health check
    pub latency: Option<std::time::Duration>,
    /// Additional metadata
    pub metadata: std::collections::HashMap<String, String>,
}

/// Health state variants
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum HealthState {
    /// Resource is fully operational
    Healthy,
    /// Resource is partially operational with degraded performance
    Degraded {
        /// Reason for degradation
        reason: String,
        /// Performance impact (0.0 = no impact, 1.0 = completely degraded)
        performance_impact: f64,
    },
    /// Resource is not operational
    Unhealthy {
        /// Reason for being unhealthy
        reason: String,
        /// Whether the resource can potentially recover
        recoverable: bool,
    },
    /// Health status is unknown
    Unknown,
}

impl HealthStatus {
    /// Create a healthy status
    #[must_use]
    pub fn healthy() -> Self {
        Self {
            state: HealthState::Healthy,
            latency: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create an unhealthy status
    pub fn unhealthy<S: Into<String>>(reason: S) -> Self {
        Self {
            state: HealthState::Unhealthy {
                reason: reason.into(),
                recoverable: true,
            },
            latency: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create a degraded status
    pub fn degraded<S: Into<String>>(reason: S, performance_impact: f64) -> Self {
        Self {
            state: HealthState::Degraded {
                reason: reason.into(),
                performance_impact: performance_impact.clamp(0.0, 1.0),
            },
            latency: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Add latency information
    #[must_use]
    pub fn with_latency(mut self, latency: std::time::Duration) -> Self {
        self.latency = Some(latency);
        self
    }

    /// Add metadata key-value pair
    pub fn with_metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Check if the resource is considered healthy enough to use
    #[must_use]
    pub fn is_usable(&self) -> bool {
        match &self.state {
            HealthState::Healthy => true,
            HealthState::Degraded {
                performance_impact, ..
            } => *performance_impact < 0.8,
            HealthState::Unhealthy { .. } | HealthState::Unknown => false,
        }
    }

    /// Get a numeric score for the health status (0.0 = unhealthy, 1.0 = healthy)
    #[must_use]
    pub fn score(&self) -> f64 {
        match &self.state {
            HealthState::Healthy => 1.0,
            HealthState::Degraded {
                performance_impact, ..
            } => 1.0 - performance_impact,
            HealthState::Unhealthy { .. } => 0.0,
            HealthState::Unknown => 0.5,
        }
    }
}

/// Trait for resources that support health checking
#[async_trait]
pub trait HealthCheckable: Send + Sync {
    /// Perform a health check on the resource
    async fn health_check(&self) -> ResourceResult<HealthStatus>;

    /// Perform a detailed health check with additional context
    async fn detailed_health_check(
        &self,
        _context: &ResourceContext,
    ) -> ResourceResult<HealthStatus> {
        // Default implementation just calls the basic health check
        self.health_check().await
    }

    /// Get the recommended interval between health checks
    fn health_check_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    /// Get the timeout for health check operations
    fn health_check_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }
}

/// Configuration for resource pooling
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PoolConfig {
    /// Minimum number of resources in the pool
    pub min_size: usize,
    /// Maximum number of resources in the pool
    pub max_size: usize,
    /// Timeout for acquiring a resource from the pool
    pub acquire_timeout: std::time::Duration,
    /// Time after which idle resources are removed
    pub idle_timeout: std::time::Duration,
    /// Maximum lifetime of a resource
    pub max_lifetime: std::time::Duration,
    /// Interval for validation/health checks
    pub validation_interval: std::time::Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
            acquire_timeout: std::time::Duration::from_secs(30),
            idle_timeout: std::time::Duration::from_secs(600),
            max_lifetime: std::time::Duration::from_secs(3600),
            validation_interval: std::time::Duration::from_secs(30),
        }
    }
}

/// Trait for resources that support pooling
pub trait Poolable: Send + Sync {
    /// Get the pool configuration for this resource type
    fn pool_config(&self) -> PoolConfig;

    /// Check if a resource instance is still valid for pooling
    fn is_valid_for_pool(&self) -> bool {
        true
    }

    /// Prepare the resource for being returned to the pool
    fn prepare_for_pool(&mut self) -> ResourceResult<()> {
        Ok(())
    }

    /// Prepare the resource for being acquired from the pool
    fn prepare_for_acquisition(&mut self) -> ResourceResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_usable() {
        assert!(HealthStatus::healthy().is_usable());
        assert!(HealthStatus::degraded("load", 0.5).is_usable());
        assert!(!HealthStatus::degraded("load", 0.9).is_usable());
        assert!(!HealthStatus::unhealthy("down").is_usable());
    }

    #[test]
    fn test_health_status_score() {
        assert_eq!(HealthStatus::healthy().score(), 1.0);
        assert_eq!(HealthStatus::degraded("load", 0.3).score(), 0.7);
        assert_eq!(HealthStatus::unhealthy("down").score(), 0.0);
        assert_eq!(
            HealthStatus {
                state: HealthState::Unknown,
                latency: None,
                metadata: std::collections::HashMap::new(),
            }
            .score(),
            0.5
        );
    }

    #[test]
    fn test_health_status_with_metadata() {
        let status = HealthStatus::healthy()
            .with_latency(std::time::Duration::from_millis(100))
            .with_metadata("version", "14.5")
            .with_metadata("connections", "10");

        assert!(status.latency.is_some());
        assert_eq!(status.metadata.get("version").unwrap(), "14.5");
        assert_eq!(status.metadata.get("connections").unwrap(), "10");
    }

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.min_size, 1);
        assert_eq!(config.max_size, 10);
        assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(30));
    }
}
