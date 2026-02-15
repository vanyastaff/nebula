//! Health checking types and background health monitoring
//!
//! This module provides:
//! - Health status types (`HealthCheckable`, `HealthStatus`, `HealthState`)
//! - Background health monitoring (`HealthChecker`)

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{context::ResourceContext, error::ResourceResult};

// ---------------------------------------------------------------------------
// Health types (from core/traits)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Background health checker (from health/mod.rs)
// ---------------------------------------------------------------------------

/// Health check record for a specific resource instance
#[derive(Debug, Clone)]
pub struct HealthRecord {
    /// Resource identifier
    pub resource_id: String,
    /// Instance identifier
    pub instance_id: uuid::Uuid,
    /// Current health status
    pub status: HealthStatus,
    /// Timestamp of the check
    pub checked_at: chrono::DateTime<chrono::Utc>,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
}

/// Configuration for background health checks
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Default interval between health checks
    pub default_interval: Duration,
    /// Number of consecutive failures before marking as unhealthy
    pub failure_threshold: u32,
    /// Whether to automatically remove unhealthy instances
    pub auto_remove_unhealthy: bool,
    /// Health check timeout
    pub check_timeout: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            default_interval: Duration::from_secs(30),
            failure_threshold: 3,
            auto_remove_unhealthy: false,
            check_timeout: Duration::from_secs(5),
        }
    }
}

/// Background health checker for resource instances
#[derive(Debug)]
pub struct HealthChecker {
    /// Configuration
    config: HealthCheckConfig,
    /// Health records by instance ID
    records: Arc<DashMap<uuid::Uuid, HealthRecord>>,
    /// Shutdown signal
    shutdown: Arc<RwLock<bool>>,
}

impl HealthChecker {
    /// Create a new health checker
    #[must_use]
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            config,
            records: Arc::new(DashMap::new()),
            shutdown: Arc::new(RwLock::new(false)),
        }
    }

    /// Start background health checking for an instance
    pub fn start_monitoring<T: HealthCheckable + 'static>(
        &self,
        instance_id: uuid::Uuid,
        resource_id: String,
        instance: Arc<T>,
    ) {
        let interval = self.config.default_interval;
        let check_timeout = self.config.check_timeout;
        let failure_threshold = self.config.failure_threshold;
        let records = Arc::clone(&self.records);
        let shutdown = Arc::clone(&self.shutdown);

        tokio::spawn(async move {
            let mut consecutive_failures = 0;

            loop {
                if *shutdown.read().await {
                    break;
                }

                // Perform health check with timeout
                let check_result =
                    tokio::time::timeout(check_timeout, instance.health_check()).await;

                let status = Self::process_check_result(
                    check_result,
                    &mut consecutive_failures,
                );

                // Record health status
                records.insert(
                    instance_id,
                    HealthRecord {
                        resource_id: resource_id.clone(),
                        instance_id,
                        status,
                        checked_at: chrono::Utc::now(),
                        consecutive_failures,
                    },
                );

                // If we've exceeded failure threshold, log warning
                if consecutive_failures >= failure_threshold {
                    tracing::warn!(
                        "Instance {} of resource {} has failed {} consecutive health checks",
                        instance_id,
                        resource_id,
                        consecutive_failures
                    );
                }

                // Wait for next check
                tokio::time::sleep(interval).await;
            }

            // Cleanup record on shutdown
            records.remove(&instance_id);
        });
    }

    /// Process a health check result, updating the consecutive failure count.
    fn process_check_result(
        result: Result<ResourceResult<HealthStatus>, tokio::time::error::Elapsed>,
        consecutive_failures: &mut u32,
    ) -> HealthStatus {
        match result {
            Ok(Ok(status)) if status.is_usable() => {
                *consecutive_failures = 0;
                status
            }
            Ok(Ok(status)) => {
                *consecutive_failures += 1;
                status
            }
            Ok(Err(e)) => {
                *consecutive_failures += 1;
                HealthStatus::unhealthy(format!("Health check failed: {e}"))
            }
            Err(_) => {
                *consecutive_failures += 1;
                HealthStatus::unhealthy("Health check timed out")
            }
        }
    }

    /// Stop monitoring an instance
    pub fn stop_monitoring(&self, instance_id: &uuid::Uuid) {
        self.records.remove(instance_id);
    }

    /// Get the current health status of an instance
    #[must_use]
    pub fn get_health(&self, instance_id: &uuid::Uuid) -> Option<HealthRecord> {
        self.records.get(instance_id).map(|r| r.value().clone())
    }

    /// Get all health records
    #[must_use]
    pub fn get_all_health(&self) -> Vec<HealthRecord> {
        self.records
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get unhealthy instances
    #[must_use]
    pub fn get_unhealthy_instances(&self) -> Vec<HealthRecord> {
        self.records
            .iter()
            .filter(|entry| !entry.value().status.is_usable())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get instances that have exceeded the failure threshold
    #[must_use]
    pub fn get_critical_instances(&self) -> Vec<HealthRecord> {
        self.records
            .iter()
            .filter(|entry| entry.value().consecutive_failures >= self.config.failure_threshold)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Shutdown the health checker
    pub async fn shutdown(&self) {
        *self.shutdown.write().await = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Health type tests ---

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

    // --- HealthChecker tests ---

    struct MockHealthCheckable {
        should_fail: Arc<RwLock<bool>>,
    }

    #[async_trait]
    impl HealthCheckable for MockHealthCheckable {
        async fn health_check(&self) -> ResourceResult<HealthStatus> {
            let should_fail = *self.should_fail.read().await;
            if should_fail {
                Ok(HealthStatus::unhealthy("mock failure"))
            } else {
                Ok(HealthStatus::healthy())
            }
        }
    }

    #[tokio::test]
    async fn test_health_checker_creation() {
        let checker = HealthChecker::new(HealthCheckConfig::default());
        assert_eq!(checker.get_all_health().len(), 0);
    }

    #[tokio::test]
    async fn test_start_monitoring() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(100),
            failure_threshold: 2,
            auto_remove_unhealthy: false,
            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let resource_id = "test".to_string();
        let should_fail = Arc::new(RwLock::new(false));
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::clone(&should_fail),
        });

        checker.start_monitoring(instance_id, resource_id.clone(), instance);

        // Wait for first check
        tokio::time::sleep(Duration::from_millis(150)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(health.status.is_usable());
        assert_eq!(health.consecutive_failures, 0);

        checker.shutdown().await;
    }

    #[tokio::test]
    async fn test_consecutive_failures() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 3,
            auto_remove_unhealthy: false,
            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let resource_id = "test".to_string();
        let should_fail = Arc::new(RwLock::new(true)); // Start failing immediately
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::clone(&should_fail),
        });

        checker.start_monitoring(instance_id, resource_id.clone(), instance);

        // Wait for multiple checks
        tokio::time::sleep(Duration::from_millis(200)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(!health.status.is_usable());
        assert!(health.consecutive_failures > 0);

        checker.shutdown().await;
    }

    #[tokio::test]
    async fn test_get_unhealthy_instances() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 2,
            auto_remove_unhealthy: false,
            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        // Add healthy instance
        let healthy_id = uuid::Uuid::new_v4();
        let healthy_instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(RwLock::new(false)),
        });
        checker.start_monitoring(healthy_id, "test".to_string(), healthy_instance);

        // Add unhealthy instance
        let unhealthy_id = uuid::Uuid::new_v4();
        let unhealthy_instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(RwLock::new(true)),
        });
        checker.start_monitoring(unhealthy_id, "test".to_string(), unhealthy_instance);

        // Wait for checks
        tokio::time::sleep(Duration::from_millis(150)).await;

        let unhealthy = checker.get_unhealthy_instances();
        assert_eq!(unhealthy.len(), 1);
        assert_eq!(unhealthy[0].instance_id, unhealthy_id);

        checker.shutdown().await;
    }
}
