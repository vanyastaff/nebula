//! Health checking types and background health monitoring
//!
//! This module provides:
//! - Health status types (`HealthCheckable`, `HealthStatus`, `HealthState`)
//! - Background health monitoring (`HealthChecker`)

use dashmap::DashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::events::{EventBus, ResourceEvent};
use crate::{context::Context, error::Result};

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
    #[must_use]
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
pub trait HealthCheckable: Send + Sync {
    /// Perform a health check on the resource
    fn health_check(&self) -> impl Future<Output = Result<HealthStatus>> + Send;

    /// Perform a detailed health check with additional context
    fn detailed_health_check(
        &self,
        _context: &Context,
    ) -> impl Future<Output = Result<HealthStatus>> + Send {
        async { self.health_check().await }
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
    /// Health check timeout
    pub check_timeout: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            default_interval: Duration::from_secs(30),
            failure_threshold: 3,
            check_timeout: Duration::from_secs(5),
        }
    }
}

/// Background health checker for resource instances
#[derive(Debug, Clone)]
pub struct HealthChecker {
    /// Configuration
    config: HealthCheckConfig,
    /// Health records by instance ID
    records: Arc<DashMap<uuid::Uuid, HealthRecord>>,
    /// Per-instance cancellation tokens (child of the global `cancel`)
    instance_tokens: Arc<DashMap<uuid::Uuid, CancellationToken>>,
    /// Cancellation token for shutdown
    cancel: CancellationToken,
    /// Optional event bus for emitting health state transitions.
    event_bus: Option<Arc<EventBus>>,
}

impl HealthChecker {
    /// Create a new health checker
    #[must_use]
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            config,
            records: Arc::new(DashMap::new()),
            instance_tokens: Arc::new(DashMap::new()),
            cancel: CancellationToken::new(),
            event_bus: None,
        }
    }

    /// Create a new health checker with an event bus for state transitions.
    #[must_use]
    pub fn with_event_bus(config: HealthCheckConfig, event_bus: Arc<EventBus>) -> Self {
        Self {
            config,
            records: Arc::new(DashMap::new()),
            instance_tokens: Arc::new(DashMap::new()),
            cancel: CancellationToken::new(),
            event_bus: Some(event_bus),
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
        let instance_tokens = Arc::clone(&self.instance_tokens);
        let event_bus = self.event_bus.clone();
        // Cancel any previous monitoring task for this instance_id
        // to avoid orphaned tasks that run until shutdown.
        if let Some((_, old_token)) = self.instance_tokens.remove(&instance_id) {
            old_token.cancel();
        }

        // Per-instance token: child of the global cancel token.
        // Cancelled by either stop_monitoring() or shutdown().
        let cancel = self.cancel.child_token();
        self.instance_tokens.insert(instance_id, cancel.clone());

        tokio::spawn(async move {
            let mut consecutive_failures = 0;
            let mut previous_state: Option<HealthState> = None;

            loop {
                // Wait for next check or cancellation
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    () = cancel.cancelled() => break,
                }

                // Perform health check with timeout
                let check_result =
                    tokio::time::timeout(check_timeout, instance.health_check()).await;

                let status = Self::process_check_result(check_result, &mut consecutive_failures);

                // Detect state transitions and emit HealthChanged events
                previous_state = Self::emit_health_transition(
                    &event_bus,
                    &resource_id,
                    &status.state,
                    previous_state,
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
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        "Instance {} of resource {} has failed {} consecutive health checks",
                        instance_id,
                        resource_id,
                        consecutive_failures
                    );
                }
            }

            // Cleanup record and token on shutdown/stop
            records.remove(&instance_id);
            instance_tokens.remove(&instance_id);
        });
    }

    /// Check if the health state changed and emit an event if so.
    ///
    /// Returns the new `previous_state` to carry forward.
    fn emit_health_transition(
        event_bus: &Option<Arc<EventBus>>,
        resource_id: &str,
        current: &HealthState,
        previous: Option<HealthState>,
    ) -> Option<HealthState> {
        let Some(bus) = event_bus else {
            return previous;
        };
        let changed = previous.as_ref().is_none_or(|prev| prev != current);
        if changed {
            let from = previous.unwrap_or(HealthState::Unknown);
            bus.emit(ResourceEvent::HealthChanged {
                resource_id: resource_id.to_string(),
                from,
                to: current.clone(),
            });
        }
        Some(current.clone())
    }

    /// Process a health check result, updating the consecutive failure count.
    fn process_check_result(
        result: std::result::Result<Result<HealthStatus>, tokio::time::error::Elapsed>,
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

    /// Stop monitoring an instance.
    ///
    /// Cancels the background monitoring task and removes the health record.
    pub fn stop_monitoring(&self, instance_id: &uuid::Uuid) {
        // Cancel the per-instance token to stop the spawned task.
        if let Some((_, token)) = self.instance_tokens.remove(instance_id) {
            token.cancel();
        }
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

    /// Shutdown the health checker, cancelling all background monitoring tasks.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ---------------------------------------------------------------------------
// HealthStage + HealthPipeline
// ---------------------------------------------------------------------------

/// A single stage in a [`HealthPipeline`].
///
/// Stages run in order. If any stage returns `Unhealthy`, the pipeline
/// short-circuits. The final result is the *worst* state observed.
pub trait HealthStage: Send + Sync {
    /// Human-readable name of this stage (for diagnostics).
    fn name(&self) -> &str;

    /// Run the health check for this stage.
    fn check(&self, ctx: &Context) -> impl Future<Output = Result<HealthStatus>> + Send;
}

/// A pipeline of [`HealthStage`]s with short-circuit semantics.
///
/// Stages are evaluated in order. If any stage returns an `Unhealthy` state
/// the pipeline stops immediately. Otherwise the worst (lowest-score) status
/// across all stages is returned.
pub struct HealthPipeline {
    stages: Vec<Box<dyn HealthStageBoxed>>,
}

/// Object-safe wrapper so we can store heterogeneous stages in a `Vec`.
trait HealthStageBoxed: Send + Sync {
    fn name(&self) -> &str;
    fn check_boxed<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<HealthStatus>> + Send + 'a>>;
}

impl<T: HealthStage> HealthStageBoxed for T {
    fn name(&self) -> &str {
        HealthStage::name(self)
    }

    fn check_boxed<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<HealthStatus>> + Send + 'a>> {
        Box::pin(HealthStage::check(self, ctx))
    }
}

impl HealthPipeline {
    /// Create a new empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    /// Add a stage to the end of the pipeline.
    pub fn add_stage<S: HealthStage + 'static>(&mut self, stage: S) {
        self.stages.push(Box::new(stage));
    }

    /// Run all stages in order with short-circuit on `Unhealthy`.
    ///
    /// Returns the worst status observed, or `Healthy` if the pipeline is
    /// empty.
    pub async fn run(&self, ctx: &Context) -> Result<HealthStatus> {
        let mut worst = HealthStatus::healthy();

        for stage in &self.stages {
            let status = stage.check_boxed(ctx).await?;
            if matches!(status.state, HealthState::Unhealthy { .. }) {
                // Short-circuit: return immediately on Unhealthy.
                return Ok(status);
            }
            if status.score() < worst.score() {
                worst = status;
            }
        }

        Ok(worst)
    }

    /// Number of stages in the pipeline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// Whether the pipeline has no stages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

impl Default for HealthPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HealthPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.stages.iter().map(|s| s.name()).collect();
        f.debug_struct("HealthPipeline")
            .field("stages", &names)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Built-in stages
// ---------------------------------------------------------------------------

/// Connectivity stage -- checks whether a resource is reachable.
///
/// Uses a caller-supplied async function to probe connectivity.
/// Returns `Healthy` when the probe returns `true`, `Unhealthy` otherwise.
pub struct ConnectivityStage<F> {
    check_fn: F,
}

impl<F> ConnectivityStage<F> {
    /// Create a new connectivity stage with the given probe function.
    ///
    /// `check_fn` receives the `execution_id` from the [`Context`] and should
    /// return `true` when the resource is reachable.
    pub fn new(check_fn: F) -> Self {
        Self { check_fn }
    }
}

impl<F, Fut> HealthStage for ConnectivityStage<F>
where
    F: Fn(&str) -> Fut + Send + Sync,
    Fut: Future<Output = bool> + Send,
{
    fn name(&self) -> &str {
        "connectivity"
    }

    async fn check(&self, ctx: &Context) -> Result<HealthStatus> {
        let reachable = (self.check_fn)(&ctx.execution_id).await;
        if reachable {
            Ok(HealthStatus::healthy())
        } else {
            Ok(HealthStatus::unhealthy("connectivity check failed"))
        }
    }
}

impl<F> std::fmt::Debug for ConnectivityStage<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectivityStage").finish_non_exhaustive()
    }
}

/// A boxed async probe function: given a resource ID, returns the measured latency.
type ProbeFn =
    dyn Fn(&str) -> std::pin::Pin<Box<dyn Future<Output = Duration> + Send>> + Send + Sync;

/// Performance stage -- checks whether latency is within acceptable bounds.
///
/// Compares the `latency` field of the *worst status seen so far* (passed via
/// the pipeline) against two thresholds:
///
/// - `warn_threshold`: latency above this is considered **Degraded**.
/// - `fail_threshold`: latency above this is considered **Unhealthy**.
///
/// If no prior stage recorded a latency, the stage returns `Healthy` (no data
/// to judge).
pub struct PerformanceStage {
    warn_threshold: Duration,
    fail_threshold: Duration,
    /// An optional probe function that measures latency on its own.
    /// When `None`, the stage returns Healthy (nothing to measure).
    probe: Option<Arc<ProbeFn>>,
}

impl Clone for PerformanceStage {
    fn clone(&self) -> Self {
        Self {
            warn_threshold: self.warn_threshold,
            fail_threshold: self.fail_threshold,
            probe: self.probe.clone(),
        }
    }
}

impl std::fmt::Debug for PerformanceStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerformanceStage")
            .field("warn_threshold", &self.warn_threshold)
            .field("fail_threshold", &self.fail_threshold)
            .field("has_probe", &self.probe.is_some())
            .finish()
    }
}

impl PerformanceStage {
    /// Create a performance stage with the given latency thresholds.
    ///
    /// `warn_threshold` must be less than `fail_threshold`.
    #[must_use]
    pub fn new(warn_threshold: Duration, fail_threshold: Duration) -> Self {
        debug_assert!(
            warn_threshold <= fail_threshold,
            "warn_threshold must be <= fail_threshold"
        );
        Self {
            warn_threshold,
            fail_threshold,
            probe: None,
        }
    }

    /// Attach an async probe function that measures latency directly.
    ///
    /// When set, the stage will call this function instead of relying on
    /// metadata from preceding stages.
    #[must_use]
    pub fn with_probe<F, Fut>(mut self, probe: F) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Duration> + Send + 'static,
    {
        self.probe = Some(Arc::new(move |id: &str| Box::pin(probe(id))));
        self
    }

    /// Evaluate latency against the configured thresholds.
    fn evaluate(&self, latency: Duration) -> HealthStatus {
        if latency >= self.fail_threshold {
            HealthStatus::unhealthy(format!(
                "latency {}ms exceeds fail threshold {}ms",
                latency.as_millis(),
                self.fail_threshold.as_millis(),
            ))
            .with_latency(latency)
        } else if latency >= self.warn_threshold {
            let impact = (latency - self.warn_threshold).as_secs_f64()
                / (self.fail_threshold - self.warn_threshold).as_secs_f64();
            HealthStatus::degraded(
                format!(
                    "latency {}ms exceeds warn threshold {}ms",
                    latency.as_millis(),
                    self.warn_threshold.as_millis(),
                ),
                impact.clamp(0.0, 1.0),
            )
            .with_latency(latency)
        } else {
            HealthStatus::healthy().with_latency(latency)
        }
    }
}

impl HealthStage for PerformanceStage {
    fn name(&self) -> &str {
        "performance"
    }

    async fn check(&self, ctx: &Context) -> Result<HealthStatus> {
        if let Some(probe) = &self.probe {
            let latency = probe(&ctx.execution_id).await;
            return Ok(self.evaluate(latency));
        }

        // No probe -- return healthy (nothing to measure).
        Ok(HealthStatus::healthy())
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

    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockHealthCheckable {
        should_fail: Arc<AtomicBool>,
    }

    impl HealthCheckable for MockHealthCheckable {
        async fn health_check(&self) -> Result<HealthStatus> {
            if self.should_fail.load(Ordering::Relaxed) {
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

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let resource_id = "test".to_string();
        let should_fail = Arc::new(AtomicBool::new(false));
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::clone(&should_fail),
        });

        checker.start_monitoring(instance_id, resource_id.clone(), instance);

        // Wait for first check
        tokio::time::sleep(Duration::from_millis(150)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(health.status.is_usable());
        assert_eq!(health.consecutive_failures, 0);

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_consecutive_failures() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 3,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let resource_id = "test".to_string();
        let should_fail = Arc::new(AtomicBool::new(true)); // Start failing immediately
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::clone(&should_fail),
        });

        checker.start_monitoring(instance_id, resource_id.clone(), instance);

        // Wait for multiple checks
        tokio::time::sleep(Duration::from_millis(200)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(!health.status.is_usable());
        assert!(health.consecutive_failures > 0);

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_shutdown_cancels_monitoring() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 2,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(false)),
        });

        checker.start_monitoring(instance_id, "test".to_string(), instance);

        // Wait for first check
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(checker.get_health(&instance_id).is_some());

        // Shutdown should cancel the monitoring task
        checker.shutdown();

        // Give the task time to notice cancellation and clean up
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Record should be removed by the shutdown cleanup in the spawned task
        assert!(
            checker.get_health(&instance_id).is_none(),
            "health record should be removed after shutdown"
        );
    }

    #[tokio::test]
    async fn test_health_check_timeout() {
        struct SlowHealthCheck;

        impl HealthCheckable for SlowHealthCheck {
            async fn health_check(&self) -> Result<HealthStatus> {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(HealthStatus::healthy())
            }
        }

        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 2,

            check_timeout: Duration::from_millis(100), // short timeout
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let instance = Arc::new(SlowHealthCheck);

        checker.start_monitoring(instance_id, "slow".to_string(), instance);

        // Wait for a couple of timed-out checks
        tokio::time::sleep(Duration::from_millis(300)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(
            !health.status.is_usable(),
            "timed-out checks should be unhealthy"
        );
        assert!(
            health.consecutive_failures > 0,
            "should have consecutive failures from timeouts"
        );

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_recovery_resets_failures() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 5,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let should_fail = Arc::new(AtomicBool::new(true));
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::clone(&should_fail),
        });

        checker.start_monitoring(instance_id, "test".to_string(), instance);

        // Let it fail for a while
        tokio::time::sleep(Duration::from_millis(150)).await;
        let health = checker.get_health(&instance_id).unwrap();
        assert!(health.consecutive_failures > 0);

        // Now make it healthy
        should_fail.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(100)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert_eq!(
            health.consecutive_failures, 0,
            "recovery should reset consecutive failures"
        );

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_stop_monitoring_removes_record() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 2,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(false)),
        });

        checker.start_monitoring(instance_id, "test".to_string(), instance);
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(checker.get_health(&instance_id).is_some());

        checker.stop_monitoring(&instance_id);
        assert!(
            checker.get_health(&instance_id).is_none(),
            "stop_monitoring should remove the health record"
        );

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_get_critical_instances() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(30),
            failure_threshold: 2,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(true)),
        });

        checker.start_monitoring(instance_id, "test".to_string(), instance);

        // Wait for enough failures to exceed threshold
        tokio::time::sleep(Duration::from_millis(150)).await;

        let critical = checker.get_critical_instances();
        assert_eq!(critical.len(), 1, "should have one critical instance");
        assert!(critical[0].consecutive_failures >= 2);

        checker.shutdown();
    }

    #[tokio::test]
    async fn test_get_unhealthy_instances() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 2,

            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        // Add healthy instance
        let healthy_id = uuid::Uuid::new_v4();
        let healthy_instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(false)),
        });
        checker.start_monitoring(healthy_id, "test".to_string(), healthy_instance);

        // Add unhealthy instance
        let unhealthy_id = uuid::Uuid::new_v4();
        let unhealthy_instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(true)),
        });
        checker.start_monitoring(unhealthy_id, "test".to_string(), unhealthy_instance);

        // Wait for checks
        tokio::time::sleep(Duration::from_millis(150)).await;

        let unhealthy = checker.get_unhealthy_instances();
        assert_eq!(unhealthy.len(), 1);
        assert_eq!(unhealthy[0].instance_id, unhealthy_id);

        checker.shutdown();
    }

    #[tokio::test]
    async fn stop_monitoring_permanently_stops_task() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(30),
            failure_threshold: 2,
            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();
        let instance = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(false)),
        });

        checker.start_monitoring(instance_id, "test".to_string(), instance);

        // Wait for at least one check to run
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(checker.get_health(&instance_id).is_some());

        checker.stop_monitoring(&instance_id);

        // Wait longer than the check interval — record must NOT reappear
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            checker.get_health(&instance_id).is_none(),
            "record must not reappear after stop_monitoring"
        );

        checker.shutdown();
    }

    #[tokio::test]
    async fn double_start_monitoring_cancels_old_task() {
        let config = HealthCheckConfig {
            default_interval: Duration::from_millis(30),
            failure_threshold: 5,
            check_timeout: Duration::from_secs(1),
        };
        let checker = HealthChecker::new(config);

        let instance_id = uuid::Uuid::new_v4();

        // First monitoring: always unhealthy
        let first = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(true)),
        });
        checker.start_monitoring(instance_id, "test".to_string(), first);
        tokio::time::sleep(Duration::from_millis(80)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(
            !health.status.is_usable(),
            "first monitor should report unhealthy"
        );

        // Second monitoring with same instance_id: always healthy
        let second = Arc::new(MockHealthCheckable {
            should_fail: Arc::new(AtomicBool::new(false)),
        });
        checker.start_monitoring(instance_id, "test".to_string(), second);
        tokio::time::sleep(Duration::from_millis(80)).await;

        let health = checker.get_health(&instance_id).unwrap();
        assert!(
            health.status.is_usable(),
            "second monitor should overwrite with healthy status"
        );
        assert_eq!(
            health.consecutive_failures, 0,
            "new task should have fresh failure counter"
        );

        checker.shutdown();
    }

    // --- HealthPipeline + built-in stage tests ---

    /// Helper: build a minimal [`Context`] for pipeline tests.
    fn test_ctx() -> Context {
        Context::new(crate::scope::Scope::Global, "wf-test", "exec-test")
    }

    /// A test stage that records whether `check` was called.
    struct SpyStage {
        label: &'static str,
        status: HealthStatus,
        called: Arc<AtomicBool>,
    }

    impl SpyStage {
        fn new(label: &'static str, status: HealthStatus) -> (Self, Arc<AtomicBool>) {
            let called = Arc::new(AtomicBool::new(false));
            (
                Self {
                    label,
                    status,
                    called: Arc::clone(&called),
                },
                called,
            )
        }
    }

    impl HealthStage for SpyStage {
        fn name(&self) -> &str {
            self.label
        }

        async fn check(&self, _ctx: &Context) -> Result<HealthStatus> {
            self.called.store(true, Ordering::SeqCst);
            Ok(self.status.clone())
        }
    }

    #[tokio::test]
    async fn test_empty_pipeline_returns_healthy() {
        let pipeline = HealthPipeline::new();
        let result = pipeline.run(&test_ctx()).await.unwrap();
        assert_eq!(result.state, HealthState::Healthy);
    }

    #[tokio::test]
    async fn test_pipeline_runs_all_healthy_stages() {
        let mut pipeline = HealthPipeline::new();

        let (stage_a, called_a) = SpyStage::new("a", HealthStatus::healthy());
        let (stage_b, called_b) = SpyStage::new("b", HealthStatus::healthy());

        pipeline.add_stage(stage_a);
        pipeline.add_stage(stage_b);

        let result = pipeline.run(&test_ctx()).await.unwrap();
        assert_eq!(result.state, HealthState::Healthy);
        assert!(called_a.load(Ordering::SeqCst), "stage a should have run");
        assert!(called_b.load(Ordering::SeqCst), "stage b should have run");
    }

    #[tokio::test]
    async fn test_pipeline_short_circuits_on_unhealthy() {
        let mut pipeline = HealthPipeline::new();

        let (unhealthy_stage, called_first) =
            SpyStage::new("unhealthy", HealthStatus::unhealthy("down"));
        let (healthy_stage, called_second) = SpyStage::new("healthy", HealthStatus::healthy());

        pipeline.add_stage(unhealthy_stage);
        pipeline.add_stage(healthy_stage);

        let result = pipeline.run(&test_ctx()).await.unwrap();
        assert!(
            matches!(result.state, HealthState::Unhealthy { .. }),
            "pipeline should return unhealthy"
        );
        assert!(
            called_first.load(Ordering::SeqCst),
            "first stage should have run"
        );
        assert!(
            !called_second.load(Ordering::SeqCst),
            "second stage should NOT have run (short-circuit)"
        );
    }

    #[tokio::test]
    async fn test_connectivity_stage_healthy() {
        let stage = ConnectivityStage::new(|_id: &str| async { true });
        let result = stage.check(&test_ctx()).await.unwrap();
        assert_eq!(result.state, HealthState::Healthy);
    }

    #[tokio::test]
    async fn test_connectivity_stage_unhealthy() {
        let stage = ConnectivityStage::new(|_id: &str| async { false });
        let result = stage.check(&test_ctx()).await.unwrap();
        assert!(
            matches!(result.state, HealthState::Unhealthy { .. }),
            "unreachable resource should be unhealthy"
        );
    }

    #[tokio::test]
    async fn test_performance_stage_degraded() {
        let stage = PerformanceStage::new(Duration::from_millis(100), Duration::from_millis(500))
            .with_probe(|_id: &str| async { Duration::from_millis(250) });

        let result = stage.check(&test_ctx()).await.unwrap();
        assert!(
            matches!(result.state, HealthState::Degraded { .. }),
            "latency between warn and fail should be degraded, got {:?}",
            result.state
        );
        assert!(result.latency.is_some());
    }

    #[tokio::test]
    async fn test_performance_stage_unhealthy() {
        let stage = PerformanceStage::new(Duration::from_millis(100), Duration::from_millis(500))
            .with_probe(|_id: &str| async { Duration::from_millis(600) });

        let result = stage.check(&test_ctx()).await.unwrap();
        assert!(
            matches!(result.state, HealthState::Unhealthy { .. }),
            "latency above fail threshold should be unhealthy, got {:?}",
            result.state
        );
        assert!(result.latency.is_some());
    }
}
