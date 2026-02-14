//! Observability and monitoring for resources

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "metrics")]
use metrics::{Counter, Gauge, Histogram};

use crate::core::{
    context::ResourceContext,
    lifecycle::{LifecycleEvent, LifecycleState},
    resource::ResourceId,
};

/// Resource metrics collector
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ResourceMetrics {
    /// Resource acquisition counter
    #[cfg(feature = "metrics")]
    acquisitions: Counter,
    /// Resource release counter
    #[cfg(feature = "metrics")]
    releases: Counter,
    /// Active resource instances gauge
    #[cfg(feature = "metrics")]
    active_instances: Gauge,
    /// Resource acquisition duration histogram
    #[cfg(feature = "metrics")]
    acquisition_duration: Histogram,
    /// Resource creation counter
    #[cfg(feature = "metrics")]
    creations: Counter,
    /// Resource destruction counter
    #[cfg(feature = "metrics")]
    destructions: Counter,
    /// Health check counter
    #[cfg(feature = "metrics")]
    health_checks: Counter,
    /// Error counter
    #[cfg(feature = "metrics")]
    errors: Counter,

    /// Labels for this resource type
    labels: HashMap<String, String>,
}

impl ResourceMetrics {
    /// Create new resource metrics
    #[must_use]
    pub fn new(resource_id: &ResourceId, labels: HashMap<String, String>) -> Self {
        let mut all_labels = labels;
        all_labels.insert("resource_id".to_string(), resource_id.unique_key());
        all_labels.insert("resource_name".to_string(), resource_id.name.clone());
        all_labels.insert("resource_version".to_string(), resource_id.version.clone());

        Self {
            #[cfg(feature = "metrics")]
            acquisitions: metrics::counter!("resource_acquisitions_total", all_labels.clone()),
            #[cfg(feature = "metrics")]
            releases: metrics::counter!("resource_releases_total", all_labels.clone()),
            #[cfg(feature = "metrics")]
            active_instances: metrics::gauge!("resource_active_instances", all_labels.clone()),
            #[cfg(feature = "metrics")]
            acquisition_duration: metrics::histogram!(
                "resource_acquisition_duration_seconds",
                all_labels.clone()
            ),
            #[cfg(feature = "metrics")]
            creations: metrics::counter!("resource_creations_total", all_labels.clone()),
            #[cfg(feature = "metrics")]
            destructions: metrics::counter!("resource_destructions_total", all_labels.clone()),
            #[cfg(feature = "metrics")]
            health_checks: metrics::counter!("resource_health_checks_total", all_labels.clone()),
            #[cfg(feature = "metrics")]
            errors: metrics::counter!("resource_errors_total", all_labels.clone()),
            labels: all_labels,
        }
    }

    /// Record a resource acquisition
    pub fn record_acquisition(&self, _duration: Duration) {
        #[cfg(feature = "metrics")]
        {
            self.acquisitions.increment(1);
            self.acquisition_duration.record(duration.as_secs_f64());
        }
    }

    /// Record a resource release
    pub fn record_release(&self) {
        #[cfg(feature = "metrics")]
        {
            self.releases.increment(1);
        }
    }

    /// Update active instances count
    pub fn set_active_instances(&self, _count: u64) {
        #[cfg(feature = "metrics")]
        {
            self.active_instances.set(count as f64);
        }
    }

    /// Record resource creation
    pub fn record_creation(&self) {
        #[cfg(feature = "metrics")]
        {
            self.creations.increment(1);
        }
    }

    /// Record resource destruction
    pub fn record_destruction(&self) {
        #[cfg(feature = "metrics")]
        {
            self.destructions.increment(1);
        }
    }

    /// Record health check
    pub fn record_health_check(&self, _success: bool) {
        #[cfg(feature = "metrics")]
        {
            let mut labels = self.labels.clone();
            labels.insert("success".to_string(), success.to_string());
            metrics::counter!("resource_health_checks_total", labels).increment(1);
        }
    }

    /// Record an error
    pub fn record_error(&self, _error_type: &str) {
        #[cfg(feature = "metrics")]
        {
            let mut labels = self.labels.clone();
            labels.insert("error_type".to_string(), error_type.to_string());
            metrics::counter!("resource_errors_total", labels).increment(1);
        }
    }

    /// Get current metrics as key-value pairs
    #[must_use]
    pub fn snapshot(&self) -> HashMap<String, f64> {
        // This would require access to the metrics registry
        // For now, return empty map
        HashMap::new()
    }
}

/// Performance metrics for resource operations
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PerformanceMetrics {
    /// Number of operations
    pub count: u64,
    /// Total duration of all operations
    pub total_duration: Duration,
    /// Minimum operation duration
    pub min_duration: Duration,
    /// Maximum operation duration
    pub max_duration: Duration,
    /// Last operation timestamp
    pub last_operation: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            count: 0,
            total_duration: Duration::ZERO,
            min_duration: Duration::MAX,
            max_duration: Duration::ZERO,
            last_operation: None,
        }
    }
}

impl PerformanceMetrics {
    /// Record a new operation
    pub fn record(&mut self, duration: Duration) {
        self.count += 1;
        self.total_duration += duration;
        self.min_duration = self.min_duration.min(duration);
        self.max_duration = self.max_duration.max(duration);
        self.last_operation = Some(chrono::Utc::now());
    }

    /// Get average duration
    #[must_use]
    pub fn average_duration(&self) -> Duration {
        if self.count == 0 {
            Duration::ZERO
        } else {
            self.total_duration / self.count as u32
        }
    }

    /// Get operations per second (based on total time span)
    #[must_use]
    pub fn ops_per_second(&self) -> f64 {
        if self.count == 0 || self.total_duration.is_zero() {
            0.0
        } else {
            self.count as f64 / self.total_duration.as_secs_f64()
        }
    }
}

/// Tracing context for resource operations
#[derive(Debug, Clone)]
pub struct TracingContext {
    /// Resource identifier
    pub resource_id: ResourceId,
    /// Operation being traced
    pub operation: String,
    /// Start time
    pub start_time: Instant,
    /// Additional attributes
    pub attributes: HashMap<String, String>,
}

impl TracingContext {
    /// Create a new tracing context
    #[must_use]
    pub fn new(resource_id: ResourceId, operation: String) -> Self {
        Self {
            resource_id,
            operation,
            start_time: Instant::now(),
            attributes: HashMap::new(),
        }
    }

    /// Add an attribute
    pub fn with_attribute<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Get the elapsed time
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Finish the trace and return the duration
    pub fn finish(self) -> Duration {
        let duration = self.elapsed();

        #[cfg(feature = "tracing")]
        {
            tracing::info!(
                resource_id = %self.resource_id,
                operation = %self.operation,
                duration_ms = duration.as_millis(),
                "Resource operation completed"
            );
        }

        duration
    }
}

/// Event emitted by the observability system
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ObservabilityEvent {
    /// Event timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Resource identifier
    pub resource_id: ResourceId,
    /// Event type
    pub event_type: ObservabilityEventType,
    /// Event data
    pub data: serde_json::Value,
    /// Resource context
    pub context: Option<ResourceContext>,
}

/// Types of observability events
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ObservabilityEventType {
    /// Resource lifecycle event
    Lifecycle(LifecycleState, LifecycleState),
    /// Performance metric event
    Performance {
        /// Operation name
        operation: String,
        /// Duration in milliseconds
        duration_ms: u64,
    },
    /// Error event
    Error {
        /// Error type
        error_type: String,
        /// Error message
        message: String,
    },
    /// Health check event
    HealthCheck {
        /// Health status
        status: String,
        /// Health score
        score: f64,
    },
    /// Custom event
    Custom {
        /// Event name
        event_name: String,
    },
}

/// Observability collector that aggregates metrics and events
pub struct ObservabilityCollector {
    /// Metrics for each resource type
    resource_metrics: Arc<RwLock<HashMap<ResourceId, ResourceMetrics>>>,
    /// Performance metrics for operations
    performance_metrics: Arc<RwLock<HashMap<String, PerformanceMetrics>>>,
    /// Event subscribers
    event_subscribers: Arc<RwLock<Vec<Box<dyn Fn(&ObservabilityEvent) + Send + Sync>>>>,
    /// Configuration
    config: ObservabilityConfig,
}

impl std::fmt::Debug for ObservabilityCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let subscriber_count = self.event_subscribers.read().len();
        f.debug_struct("ObservabilityCollector")
            .field("resource_metrics", &self.resource_metrics)
            .field("performance_metrics", &self.performance_metrics)
            .field("subscriber_count", &subscriber_count)
            .field("config", &self.config)
            .finish()
    }
}

/// Configuration for observability
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Whether to collect metrics
    pub metrics_enabled: bool,
    /// Whether to emit events
    pub events_enabled: bool,
    /// Whether to use distributed tracing
    pub tracing_enabled: bool,
    /// Metrics collection interval
    pub metrics_interval: Duration,
    /// Maximum number of events to buffer
    pub max_event_buffer: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics_enabled: true,
            events_enabled: true,
            tracing_enabled: true,
            metrics_interval: Duration::from_secs(10),
            max_event_buffer: 1000,
        }
    }
}

impl ObservabilityCollector {
    /// Create a new observability collector
    #[must_use]
    pub fn new(config: ObservabilityConfig) -> Self {
        Self {
            resource_metrics: Arc::new(RwLock::new(HashMap::new())),
            performance_metrics: Arc::new(RwLock::new(HashMap::new())),
            event_subscribers: Arc::new(RwLock::new(Vec::new())),
            config,
        }
    }

    /// Get or create metrics for a resource type
    #[must_use]
    pub fn get_resource_metrics(&self, resource_id: &ResourceId) -> ResourceMetrics {
        let metrics = self.resource_metrics.read();
        if let Some(m) = metrics.get(resource_id) {
            return m.clone();
        }
        drop(metrics);

        let mut metrics = self.resource_metrics.write();
        metrics
            .entry(resource_id.clone())
            .or_insert_with(|| ResourceMetrics::new(resource_id, HashMap::new()))
            .clone()
    }

    /// Record a lifecycle event
    pub fn record_lifecycle_event(&self, event: &LifecycleEvent) {
        if self.config.events_enabled {
            let obs_event = ObservabilityEvent {
                timestamp: event.timestamp,
                resource_id: ResourceId::new(event.resource_id.clone(), "1.0".to_string()), // Simplified
                event_type: ObservabilityEventType::Lifecycle(event.from_state, event.to_state),
                data: event.metadata.clone().unwrap_or_default(),
                context: None,
            };

            self.emit_event(&obs_event);
        }
    }

    /// Record a performance event
    pub fn record_performance(&self, operation: &str, duration: Duration) {
        if self.config.metrics_enabled {
            let mut metrics = self.performance_metrics.write();
            metrics
                .entry(operation.to_string())
                .or_default()
                .record(duration);
        }
    }

    /// Record an error event
    pub fn record_error(&self, resource_id: &ResourceId, error_type: &str, message: &str) {
        if self.config.events_enabled {
            let obs_event = ObservabilityEvent {
                timestamp: chrono::Utc::now(),
                resource_id: resource_id.clone(),
                event_type: ObservabilityEventType::Error {
                    error_type: error_type.to_string(),
                    message: message.to_string(),
                },
                data: serde_json::json!({
                    "error_type": error_type,
                    "message": message
                }),
                context: None,
            };

            self.emit_event(&obs_event);
        }
    }

    /// Subscribe to observability events
    pub fn subscribe<F>(&self, callback: F)
    where
        F: Fn(&ObservabilityEvent) + Send + Sync + 'static,
    {
        let mut subscribers = self.event_subscribers.write();
        subscribers.push(Box::new(callback));
    }

    /// Get performance metrics summary
    #[must_use]
    pub fn performance_summary(&self) -> HashMap<String, PerformanceMetrics> {
        self.performance_metrics.read().clone()
    }

    /// Start tracing an operation
    #[must_use]
    pub fn start_trace(&self, resource_id: ResourceId, operation: String) -> TracingContext {
        TracingContext::new(resource_id, operation)
    }

    /// Get observability statistics
    #[must_use]
    pub fn stats(&self) -> ObservabilityStats {
        let resource_metrics = self.resource_metrics.read();
        let performance_metrics = self.performance_metrics.read();

        ObservabilityStats {
            tracked_resources: resource_metrics.len(),
            tracked_operations: performance_metrics.len(),
            events_enabled: self.config.events_enabled,
            metrics_enabled: self.config.metrics_enabled,
            tracing_enabled: self.config.tracing_enabled,
        }
    }

    fn emit_event(&self, event: &ObservabilityEvent) {
        let subscribers = self.event_subscribers.read();
        for subscriber in subscribers.iter() {
            subscriber(event);
        }
    }
}

impl Default for ObservabilityCollector {
    fn default() -> Self {
        Self::new(ObservabilityConfig::default())
    }
}

/// Statistics about the observability system
#[derive(Debug, Clone)]
pub struct ObservabilityStats {
    /// Number of tracked resource types
    pub tracked_resources: usize,
    /// Number of tracked operations
    pub tracked_operations: usize,
    /// Whether events are enabled
    pub events_enabled: bool,
    /// Whether metrics are enabled
    pub metrics_enabled: bool,
    /// Whether tracing is enabled
    pub tracing_enabled: bool,
}

/// Macros for convenient tracing
#[macro_export]
/// Macro to trace a resource operation with automatic timing
macro_rules! trace_resource_operation {
    ($collector:expr, $resource_id:expr, $operation:expr, $code:block) => {{
        let trace = $collector.start_trace($resource_id.clone(), $operation.to_string());
        let result = $code;
        let duration = trace.finish();
        $collector.record_performance($operation, duration);
        result
    }};
}

#[macro_export]
macro_rules! record_resource_error {
    ($collector:expr, $resource_id:expr, $error_type:expr, $error:expr) => {
        $collector.record_error($resource_id, $error_type, &$error.to_string());
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_metrics() {
        let mut metrics = PerformanceMetrics::default();

        metrics.record(Duration::from_millis(100));
        metrics.record(Duration::from_millis(200));
        metrics.record(Duration::from_millis(300));

        assert_eq!(metrics.count, 3);
        assert_eq!(metrics.average_duration(), Duration::from_millis(200));
        assert_eq!(metrics.min_duration, Duration::from_millis(100));
        assert_eq!(metrics.max_duration, Duration::from_millis(300));
    }

    #[test]
    fn test_tracing_context() {
        let resource_id = ResourceId::new("test", "1.0");
        let trace = TracingContext::new(resource_id, "test_operation".to_string())
            .with_attribute("key", "value");

        assert_eq!(trace.operation, "test_operation");
        assert_eq!(trace.attributes.get("key"), Some(&"value".to_string()));

        // Test elapsed time (should be very small)
        assert!(trace.elapsed() < Duration::from_millis(10));
    }

    #[test]
    fn test_observability_collector() {
        let collector = ObservabilityCollector::new(ObservabilityConfig::default());
        let resource_id = ResourceId::new("test", "1.0");

        // Test metrics creation
        let metrics = collector.get_resource_metrics(&resource_id);
        metrics.record_acquisition(Duration::from_millis(100));

        // Test performance recording
        collector.record_performance("test_op", Duration::from_millis(50));

        let stats = collector.stats();
        assert_eq!(stats.tracked_resources, 1);
        assert_eq!(stats.tracked_operations, 1);
        assert!(stats.metrics_enabled);
    }

    #[test]
    fn test_event_subscription() {
        let collector = ObservabilityCollector::new(ObservabilityConfig::default());

        // This is a simplified test - in reality we'd need thread-safe collection
        collector.subscribe(|_event| {
            // In a real test, we'd use a thread-safe way to collect events
        });

        let resource_id = ResourceId::new("test", "1.0");
        collector.record_error(&resource_id, "test_error", "test message");
    }
}
