//! Background health checking system
//!
//! Provides automated health monitoring for resources with:
//! - Periodic health checks
//! - Configurable intervals
//! - Health status history
//! - Automatic unhealthy resource detection

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::ResourceId,
    traits::{HealthCheckable, HealthStatus},
};
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Health check record for a specific resource instance
#[derive(Debug, Clone)]
pub struct HealthRecord {
    /// Resource identifier
    pub resource_id: ResourceId,
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
        resource_id: ResourceId,
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
                // Check shutdown signal
                {
                    let should_shutdown = *shutdown.read().await;
                    if should_shutdown {
                        break;
                    }
                }

                // Perform health check with timeout
                let check_result =
                    tokio::time::timeout(check_timeout, instance.health_check()).await;

                let status = match check_result {
                    Ok(Ok(status)) => {
                        if status.is_usable() {
                            consecutive_failures = 0;
                        } else {
                            consecutive_failures += 1;
                        }
                        status
                    }
                    Ok(Err(e)) => {
                        consecutive_failures += 1;
                        HealthStatus::unhealthy(format!("Health check failed: {}", e))
                    }
                    Err(_) => {
                        consecutive_failures += 1;
                        HealthStatus::unhealthy("Health check timed out")
                    }
                };

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

    /// Stop monitoring an instance
    pub fn stop_monitoring(&self, instance_id: &uuid::Uuid) {
        self.records.remove(instance_id);
    }

    /// Get the current health status of an instance
    pub fn get_health(&self, instance_id: &uuid::Uuid) -> Option<HealthRecord> {
        self.records.get(instance_id).map(|r| r.value().clone())
    }

    /// Get all health records
    pub fn get_all_health(&self) -> Vec<HealthRecord> {
        self.records
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get unhealthy instances
    pub fn get_unhealthy_instances(&self) -> Vec<HealthRecord> {
        self.records
            .iter()
            .filter(|entry| !entry.value().status.is_usable())
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get instances that have exceeded the failure threshold
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
    use crate::core::context::ResourceContext;

    struct MockHealthCheckable {
        should_fail: Arc<RwLock<bool>>,
    }

    #[async_trait::async_trait]
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
        let resource_id = ResourceId::new("test", "1.0");
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
        let resource_id = ResourceId::new("test", "1.0");
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
        checker.start_monitoring(healthy_id, ResourceId::new("test", "1.0"), healthy_instance);

        // Add unhealthy instance
        let unhealthy_id = uuid::Uuid::new_v4();
        let unhealthy_instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(RwLock::new(true)),
        });
        checker.start_monitoring(
            unhealthy_id,
            ResourceId::new("test", "1.0"),
            unhealthy_instance,
        );

        // Wait for checks
        tokio::time::sleep(Duration::from_millis(150)).await;

        let unhealthy = checker.get_unhealthy_instances();
        assert_eq!(unhealthy.len(), 1);
        assert_eq!(unhealthy[0].instance_id, unhealthy_id);

        checker.shutdown().await;
    }
}
