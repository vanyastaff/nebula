//! Pool health monitoring and leak detection

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

#[cfg(feature = "std")]
use std::time::{Duration, Instant};

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Pool health status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolHealth {
    /// Pool is operating normally
    Healthy,
    /// Pool is experiencing degraded performance
    Degraded,
    /// Pool is experiencing critical issues
    Critical,
    /// Pool has failed
    Failed,
}

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Maximum acceptable failure rate (0.0 - 1.0)
    pub max_failure_rate: f64,

    /// Maximum acceptable leak rate (0.0 - 1.0)
    pub max_leak_rate: f64,

    /// Minimum acceptable pool utilization (0.0 - 1.0)
    pub min_utilization: f64,

    /// Maximum acceptable pool utilization (0.0 - 1.0)
    pub max_utilization: f64,

    /// Time window for rate calculations
    #[cfg(feature = "std")]
    pub rate_window: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            max_failure_rate: 0.1,      // 10% failures
            max_leak_rate: 0.05,         // 5% leaks
            min_utilization: 0.2,        // 20% minimum
            max_utilization: 0.9,        // 90% maximum
            #[cfg(feature = "std")]
            rate_window: Duration::from_secs(60),
        }
    }
}

/// Pool health monitor
pub struct PoolHealthMonitor {
    config: HealthConfig,

    // Counters
    total_checkouts: AtomicU64,
    total_returns: AtomicU64,
    total_failures: AtomicU64,
    leaked_objects: AtomicU64,

    // Pool state
    pool_capacity: AtomicUsize,
    available_objects: AtomicUsize,

    // Timing
    #[cfg(feature = "std")]
    last_check: parking_lot::Mutex<Instant>,
}

impl PoolHealthMonitor {
    /// Create new health monitor
    pub fn new(config: HealthConfig, capacity: usize) -> Self {
        Self {
            config,
            total_checkouts: AtomicU64::new(0),
            total_returns: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            leaked_objects: AtomicU64::new(0),
            pool_capacity: AtomicUsize::new(capacity),
            available_objects: AtomicUsize::new(0),
            #[cfg(feature = "std")]
            last_check: parking_lot::Mutex::new(Instant::now()),
        }
    }

    /// Record a successful checkout
    pub fn record_checkout(&self) {
        self.total_checkouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a successful return
    pub fn record_return(&self) {
        self.total_returns.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a failure (checkout failed)
    pub fn record_failure(&self) {
        self.total_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a leaked object
    pub fn record_leak(&self) {
        self.leaked_objects.fetch_add(1, Ordering::Relaxed);
    }

    /// Update available objects count
    pub fn update_available(&self, count: usize) {
        self.available_objects.store(count, Ordering::Relaxed);
    }

    /// Update pool capacity
    pub fn update_capacity(&self, capacity: usize) {
        self.pool_capacity.store(capacity, Ordering::Relaxed);
    }

    /// Get current health status
    pub fn check_health(&self) -> PoolHealth {
        let checkouts = self.total_checkouts.load(Ordering::Relaxed);
        let failures = self.total_failures.load(Ordering::Relaxed);
        let leaks = self.leaked_objects.load(Ordering::Relaxed);
        let capacity = self.pool_capacity.load(Ordering::Relaxed);
        let available = self.available_objects.load(Ordering::Relaxed);

        // Check failure rate
        let failure_rate = if checkouts > 0 {
            failures as f64 / checkouts as f64
        } else {
            0.0
        };

        if failure_rate > self.config.max_failure_rate {
            return PoolHealth::Critical;
        }

        // Check leak rate
        let leak_rate = if checkouts > 0 {
            leaks as f64 / checkouts as f64
        } else {
            0.0
        };

        if leak_rate > self.config.max_leak_rate {
            return PoolHealth::Degraded;
        }

        // Check utilization
        let utilization = if capacity > 0 {
            (capacity - available) as f64 / capacity as f64
        } else {
            0.0
        };

        if utilization < self.config.min_utilization || utilization > self.config.max_utilization {
            return PoolHealth::Degraded;
        }

        PoolHealth::Healthy
    }

    /// Detect potential leaks
    pub fn detect_leaks(&self) -> LeakDetectionReport {
        let checkouts = self.total_checkouts.load(Ordering::Relaxed);
        let returns = self.total_returns.load(Ordering::Relaxed);
        let known_leaks = self.leaked_objects.load(Ordering::Relaxed);

        let potential_leaks = checkouts.saturating_sub(returns).saturating_sub(known_leaks);

        LeakDetectionReport {
            total_checkouts: checkouts,
            total_returns: returns,
            known_leaks,
            potential_leaks,
            leak_rate: if checkouts > 0 {
                (known_leaks + potential_leaks) as f64 / checkouts as f64
            } else {
                0.0
            },
        }
    }

    /// Get health metrics
    pub fn metrics(&self) -> HealthMetrics {
        HealthMetrics {
            total_checkouts: self.total_checkouts.load(Ordering::Relaxed),
            total_returns: self.total_returns.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            leaked_objects: self.leaked_objects.load(Ordering::Relaxed),
            pool_capacity: self.pool_capacity.load(Ordering::Relaxed),
            available_objects: self.available_objects.load(Ordering::Relaxed),
            health_status: self.check_health(),
        }
    }

    /// Reset counters
    pub fn reset(&self) {
        self.total_checkouts.store(0, Ordering::Relaxed);
        self.total_returns.store(0, Ordering::Relaxed);
        self.total_failures.store(0, Ordering::Relaxed);
        self.leaked_objects.store(0, Ordering::Relaxed);
        #[cfg(feature = "std")]
        {
            *self.last_check.lock() = Instant::now();
        }
    }
}

/// Leak detection report
#[derive(Debug, Clone)]
pub struct LeakDetectionReport {
    /// Total checkouts
    pub total_checkouts: u64,

    /// Total returns
    pub total_returns: u64,

    /// Known leaked objects
    pub known_leaks: u64,

    /// Potential leaks (checkouts - returns - known_leaks)
    pub potential_leaks: u64,

    /// Leak rate (0.0 - 1.0)
    pub leak_rate: f64,
}

impl LeakDetectionReport {
    /// Check if leaks are detected
    pub fn has_leaks(&self) -> bool {
        self.known_leaks > 0 || self.potential_leaks > 0
    }

    /// Get total suspected leaks
    pub fn total_leaks(&self) -> u64 {
        self.known_leaks + self.potential_leaks
    }
}

/// Health metrics snapshot
#[derive(Debug, Clone)]
pub struct HealthMetrics {
    pub total_checkouts: u64,
    pub total_returns: u64,
    pub total_failures: u64,
    pub leaked_objects: u64,
    pub pool_capacity: usize,
    pub available_objects: usize,
    pub health_status: PoolHealth,
}

impl HealthMetrics {
    /// Get pool utilization (0.0 - 1.0)
    pub fn utilization(&self) -> f64 {
        if self.pool_capacity > 0 {
            (self.pool_capacity - self.available_objects) as f64 / self.pool_capacity as f64
        } else {
            0.0
        }
    }

    /// Get failure rate (0.0 - 1.0)
    pub fn failure_rate(&self) -> f64 {
        if self.total_checkouts > 0 {
            self.total_failures as f64 / self.total_checkouts as f64
        } else {
            0.0
        }
    }

    /// Check if pool is healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self.health_status, PoolHealth::Healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_monitor_basic() {
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 100);

        monitor.record_checkout();
        monitor.record_return();
        monitor.update_available(99);

        let health = monitor.check_health();
        assert_eq!(health, PoolHealth::Healthy);
    }

    #[test]
    fn test_leak_detection() {
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 100);

        monitor.record_checkout();
        monitor.record_checkout();
        monitor.record_return();

        let report = monitor.detect_leaks();
        assert_eq!(report.total_checkouts, 2);
        assert_eq!(report.total_returns, 1);
        assert_eq!(report.potential_leaks, 1);
        assert!(report.has_leaks());
    }

    #[test]
    fn test_health_degraded_on_high_failure_rate() {
        let config = HealthConfig {
            max_failure_rate: 0.1,
            ..Default::default()
        };

        let monitor = PoolHealthMonitor::new(config, 100);

        for _ in 0..10 {
            monitor.record_checkout();
        }

        for _ in 0..2 {
            monitor.record_failure();
        }

        let health = monitor.check_health();
        assert_eq!(health, PoolHealth::Critical);
    }

    #[test]
    fn test_metrics() {
        let monitor = PoolHealthMonitor::new(HealthConfig::default(), 100);

        monitor.record_checkout();
        monitor.update_available(50);

        let metrics = monitor.metrics();
        assert_eq!(metrics.total_checkouts, 1);
        assert_eq!(metrics.pool_capacity, 100);
        assert_eq!(metrics.available_objects, 50);
        assert_eq!(metrics.utilization(), 0.5);
    }
}
