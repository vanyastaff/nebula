//! Observability hooks for pattern lifecycle events
//!
//! This module provides type-safe observability hooks using advanced patterns:
//!
//! - **Typed event categories** for compile-time event classification
//! - **Sealed hook traits** for controlled extensibility
//! - **Const generic metric dimensions** for zero-cost abstractions
//!
//! # Typed Event Example
//!
//! ```rust,ignore
//! use nebula_resilience::observability::{Event, RetryEventCategory};
//!
//! // Type-safe event with compile-time category
//! let event = Event::<RetryEventCategory>::new("api_call")
//!     .with_attempt(1)
//!     .with_max_attempts(3);
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// SEALED EVENT CATEGORY TRAIT
// =============================================================================

mod sealed {
    pub trait SealedEventCategory {}
}

/// Event category marker trait (sealed for controlled extensibility).
pub trait EventCategory: sealed::SealedEventCategory + Send + Sync + 'static {
    /// Category name for metrics/logging.
    fn name() -> &'static str;

    /// Category description.
    fn description() -> &'static str;

    /// Default log level for this category.
    fn default_log_level() -> LogLevel {
        LogLevel::Info
    }

    /// Whether events in this category should be sampled.
    fn is_sampled() -> bool {
        false
    }
}

// =============================================================================
// EVENT CATEGORY MARKERS
// =============================================================================

/// Retry event category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryEventCategory;

impl sealed::SealedEventCategory for RetryEventCategory {}

impl EventCategory for RetryEventCategory {
    fn name() -> &'static str {
        "retry"
    }

    fn description() -> &'static str {
        "Retry pattern events"
    }
}

/// Circuit breaker event category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerEventCategory;

impl sealed::SealedEventCategory for CircuitBreakerEventCategory {}

impl EventCategory for CircuitBreakerEventCategory {
    fn name() -> &'static str {
        "circuit_breaker"
    }

    fn description() -> &'static str {
        "Circuit breaker state change events"
    }

    fn default_log_level() -> LogLevel {
        LogLevel::Warn
    }
}

/// Timeout event category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutEventCategory;

impl sealed::SealedEventCategory for TimeoutEventCategory {}

impl EventCategory for TimeoutEventCategory {
    fn name() -> &'static str {
        "timeout"
    }

    fn description() -> &'static str {
        "Timeout events"
    }

    fn default_log_level() -> LogLevel {
        LogLevel::Warn
    }
}

/// Rate limiter event category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimiterEventCategory;

impl sealed::SealedEventCategory for RateLimiterEventCategory {}

impl EventCategory for RateLimiterEventCategory {
    fn name() -> &'static str {
        "rate_limiter"
    }

    fn description() -> &'static str {
        "Rate limiting events"
    }

    fn is_sampled() -> bool {
        true // Rate limit events can be high-volume
    }
}

/// Bulkhead event category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkheadEventCategory;

impl sealed::SealedEventCategory for BulkheadEventCategory {}

impl EventCategory for BulkheadEventCategory {
    fn name() -> &'static str {
        "bulkhead"
    }

    fn description() -> &'static str {
        "Bulkhead concurrency events"
    }
}

// =============================================================================
// TYPED EVENT
// =============================================================================

/// event with compile-time category.
#[derive(Debug, Clone)]
pub struct Event<C: EventCategory> {
    /// Operation name.
    pub operation: String,
    /// Event timestamp.
    pub timestamp: std::time::Instant,
    /// Optional duration.
    pub duration: Option<Duration>,
    /// Optional error message.
    pub error: Option<String>,
    /// Additional context.
    pub context: std::collections::HashMap<String, String>,
    _category: PhantomData<C>,
}

impl<C: EventCategory> Event<C> {
    /// Create a new typed event.
    pub fn new(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            timestamp: std::time::Instant::now(),
            duration: None,
            error: None,
            context: std::collections::HashMap::new(),
            _category: PhantomData,
        }
    }

    /// Set duration.
    #[must_use]
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set error message.
    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Add context.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Get category name.
    pub fn category(&self) -> &'static str {
        C::name()
    }

    /// Get default log level.
    pub fn log_level(&self) -> LogLevel {
        C::default_log_level()
    }

    /// Check if event should be sampled.
    pub fn is_sampled(&self) -> bool {
        C::is_sampled()
    }

    /// Check if this is an error event.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

// =============================================================================
// TYPED METRIC DIMENSIONS
// =============================================================================

/// metric with const generic dimensions.
#[derive(Debug, Clone)]
pub struct Metric<const DIMENSIONS: usize> {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: f64,
    /// Dimension labels.
    pub labels: [(&'static str, String); DIMENSIONS],
}

impl<const DIMENSIONS: usize> Metric<DIMENSIONS> {
    /// Create a new typed metric.
    pub fn new(
        name: impl Into<String>,
        value: f64,
        labels: [(&'static str, String); DIMENSIONS],
    ) -> Self {
        Self {
            name: name.into(),
            value,
            labels,
        }
    }
}

/// Common metric types with predefined dimensions.
pub mod metrics {
    use super::*;

    /// Counter metric (1 dimension: name).
    pub type Counter = Metric<1>;

    /// Gauge metric (2 dimensions: name, service).
    pub type ServiceGauge = Metric<2>;

    /// Histogram metric (3 dimensions: name, service, operation).
    pub type OperationHistogram = Metric<3>;

    /// Create a counter increment.
    pub fn counter(name: &str) -> Counter {
        Metric::new(name, 1.0, [("name", name.to_string())])
    }

    /// Create a service gauge.
    pub fn service_gauge(name: &str, service: &str, value: f64) -> ServiceGauge {
        Metric::new(
            name,
            value,
            [("name", name.to_string()), ("service", service.to_string())],
        )
    }

    /// Create an operation histogram entry.
    pub fn operation_histogram(
        name: &str,
        service: &str,
        operation: &str,
        value: f64,
    ) -> OperationHistogram {
        Metric::new(
            name,
            value,
            [
                ("name", name.to_string()),
                ("service", service.to_string()),
                ("operation", operation.to_string()),
            ],
        )
    }
}

// =============================================================================
// TYPED HOOK TRAIT
// =============================================================================

/// observability hook for specific event categories.
pub trait ObservabilityHookExt<C: EventCategory>: Send + Sync {
    /// Handle a typed event.
    fn on_typed_event(&self, event: &Event<C>);
}

/// Adapter to convert ObservabilityHookExt to ObservabilityHook.
pub struct HookAdapter<C: EventCategory, H: ObservabilityHookExt<C>> {
    #[allow(dead_code)]
    hook: H,
    _category: PhantomData<C>,
}

impl<C: EventCategory, H: ObservabilityHookExt<C>> HookAdapter<C, H> {
    /// Create a new adapter.
    pub fn new(hook: H) -> Self {
        Self {
            hook,
            _category: PhantomData,
        }
    }
}

// =============================================================================
// ORIGINAL IMPLEMENTATION (with enhancements)
// =============================================================================

/// Log level for observability hooks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Error level
    Error,
    /// Warning level
    Warn,
    /// Info level
    Info,
    /// Debug level
    Debug,
    /// Trace level
    Trace,
}

/// Events that can occur during resilience pattern execution
#[derive(Debug, Clone)]
pub enum PatternEvent {
    /// Operation started
    Started {
        /// Pattern name (retry, `circuit_breaker`, etc.)
        pattern: String,
        /// Service or operation name
        operation: String,
    },
    /// Operation completed successfully
    Succeeded {
        /// Pattern name
        pattern: String,
        /// Service or operation name
        operation: String,
        /// Duration of the operation
        duration: Duration,
    },
    /// Operation failed
    Failed {
        /// Pattern name
        pattern: String,
        /// Service or operation name
        operation: String,
        /// Error that occurred
        error: String,
        /// Duration before failure
        duration: Duration,
    },
    /// Retry attempt
    RetryAttempt {
        /// Operation name
        operation: String,
        /// Attempt number (1-based)
        attempt: usize,
        /// Total attempts allowed
        max_attempts: usize,
    },
    /// Circuit breaker state changed
    CircuitBreakerStateChanged {
        /// Service name
        service: String,
        /// Old state
        from_state: String,
        /// New state
        to_state: String,
    },
    /// Rate limit exceeded
    RateLimitExceeded {
        /// Service name
        service: String,
        /// Current rate
        current_rate: f64,
    },
    /// Bulkhead capacity reached
    BulkheadCapacityReached {
        /// Service name
        service: String,
        /// Current active operations
        active: usize,
        /// Maximum capacity
        capacity: usize,
    },
    /// Timeout occurred
    TimeoutOccurred {
        /// Operation name
        operation: String,
        /// Timeout duration
        timeout: Duration,
    },
}

impl fmt::Display for PatternEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started { pattern, operation } => {
                write!(f, "{pattern} started for {operation}")
            }
            Self::Succeeded {
                pattern,
                operation,
                duration,
            } => {
                write!(f, "{pattern} succeeded for {operation} in {duration:?}")
            }
            Self::Failed {
                pattern,
                operation,
                error,
                duration,
            } => {
                write!(
                    f,
                    "{pattern} failed for {operation} after {duration:?}: {error}"
                )
            }
            Self::RetryAttempt {
                operation,
                attempt,
                max_attempts,
            } => {
                write!(f, "Retry attempt {attempt}/{max_attempts} for {operation}")
            }
            Self::CircuitBreakerStateChanged {
                service,
                from_state,
                to_state,
            } => {
                write!(
                    f,
                    "Circuit breaker for {service} changed from {from_state} to {to_state}"
                )
            }
            Self::RateLimitExceeded {
                service,
                current_rate,
            } => {
                write!(
                    f,
                    "Rate limit exceeded for {service} at {current_rate} req/s"
                )
            }
            Self::BulkheadCapacityReached {
                service,
                active,
                capacity,
            } => {
                write!(
                    f,
                    "Bulkhead capacity reached for {service}: {active}/{capacity}"
                )
            }
            Self::TimeoutOccurred { operation, timeout } => {
                write!(f, "Timeout occurred for {operation} after {timeout:?}")
            }
        }
    }
}

/// Trait for observability hooks
pub trait ObservabilityHook: Send + Sync {
    /// Called when a pattern event occurs
    fn on_event(&self, event: &PatternEvent);

    /// Called when metrics should be exported
    fn export_metrics(&self) {}

    /// Called when the hook is initialized
    fn initialize(&self) {}

    /// Called when the hook is shut down
    fn shutdown(&self) {}
}

/// Collection of observability hooks
#[derive(Default, Clone)]
pub struct ObservabilityHooks {
    hooks: Arc<Vec<Arc<dyn ObservabilityHook>>>,
}

impl ObservabilityHooks {
    /// Create a new collection of hooks
    #[must_use]
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(Vec::new()),
        }
    }

    /// Add a hook to the collection
    #[must_use]
    pub fn with_hook(mut self, hook: Arc<dyn ObservabilityHook>) -> Self {
        let hooks = Arc::make_mut(&mut self.hooks);
        hooks.push(hook);
        self
    }

    /// Emit an event to all hooks
    pub fn emit(&self, event: PatternEvent) {
        for hook in self.hooks.iter() {
            hook.on_event(&event);
        }
    }

    /// Initialize all hooks
    pub fn initialize(&self) {
        for hook in self.hooks.iter() {
            hook.initialize();
        }
    }

    /// Shutdown all hooks
    pub fn shutdown(&self) {
        for hook in self.hooks.iter() {
            hook.shutdown();
        }
    }

    /// Export metrics from all hooks
    pub fn export_metrics(&self) {
        for hook in self.hooks.iter() {
            hook.export_metrics();
        }
    }
}

/// Logging hook that outputs events using nebula-log
pub struct LoggingHook {
    level: LogLevel,
}

impl LoggingHook {
    /// Create a new logging hook
    #[must_use]
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }
}

impl ObservabilityHook for LoggingHook {
    fn on_event(&self, event: &PatternEvent) {
        match self.level {
            LogLevel::Error => nebula_log::error!("{}", event),
            LogLevel::Warn => nebula_log::warn!("{}", event),
            LogLevel::Info => nebula_log::info!("{}", event),
            LogLevel::Debug => nebula_log::debug!("{}", event),
            LogLevel::Trace => nebula_log::debug!("{}", event),
        }
    }
}

/// Metrics hook that records events as metrics
pub struct MetricsHook {
    collector: crate::core::MetricsCollector,
}

impl MetricsHook {
    /// Create a new metrics hook
    #[must_use]
    pub fn new() -> Self {
        Self {
            collector: crate::core::MetricsCollector::new(true),
        }
    }

    /// Get a snapshot of all metrics
    #[must_use]
    pub fn metrics(&self) -> std::collections::HashMap<String, crate::core::MetricSnapshot> {
        self.collector.all_metrics()
    }
}

impl Default for MetricsHook {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservabilityHook for MetricsHook {
    fn on_event(&self, event: &PatternEvent) {
        match event {
            PatternEvent::Started { pattern, .. } => {
                self.collector.increment(format!("{pattern}.started"));
            }
            PatternEvent::Succeeded {
                pattern, duration, ..
            } => {
                self.collector.increment(format!("{pattern}.success"));
                self.collector
                    .record_duration(format!("{pattern}.duration"), *duration);
            }
            PatternEvent::Failed { pattern, .. } => {
                self.collector.increment(format!("{pattern}.failure"));
            }
            PatternEvent::RetryAttempt { .. } => {
                self.collector.increment("retry.attempts");
            }
            PatternEvent::CircuitBreakerStateChanged {
                service, to_state, ..
            } => {
                self.collector
                    .increment(format!("circuit_breaker.{service}.state.{to_state}"));
            }
            PatternEvent::RateLimitExceeded { service, .. } => {
                self.collector
                    .increment(format!("rate_limit.{service}.exceeded"));
            }
            PatternEvent::BulkheadCapacityReached { service, .. } => {
                self.collector
                    .increment(format!("bulkhead.{service}.capacity_reached"));
            }
            PatternEvent::TimeoutOccurred { operation, .. } => {
                self.collector.increment(format!("timeout.{operation}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_event_display() {
        let event = PatternEvent::Started {
            pattern: "retry".to_string(),
            operation: "api_call".to_string(),
        };
        assert_eq!(event.to_string(), "retry started for api_call");

        let event = PatternEvent::Succeeded {
            pattern: "circuit_breaker".to_string(),
            operation: "db_query".to_string(),
            duration: Duration::from_millis(100),
        };
        assert!(event.to_string().contains("circuit_breaker succeeded"));
    }

    #[test]
    fn test_observability_hooks() {
        let hooks = ObservabilityHooks::new()
            .with_hook(Arc::new(MetricsHook::new()))
            .with_hook(Arc::new(LoggingHook::new(LogLevel::Info)));

        hooks.emit(PatternEvent::Started {
            pattern: "test".to_string(),
            operation: "test_op".to_string(),
        });

        hooks.emit(PatternEvent::Succeeded {
            pattern: "test".to_string(),
            operation: "test_op".to_string(),
            duration: Duration::from_millis(50),
        });
    }

    #[test]
    fn test_metrics_hook() {
        let hook = MetricsHook::new();

        hook.on_event(&PatternEvent::Started {
            pattern: "retry".to_string(),
            operation: "test".to_string(),
        });

        hook.on_event(&PatternEvent::Succeeded {
            pattern: "retry".to_string(),
            operation: "test".to_string(),
            duration: Duration::from_millis(100),
        });

        let metrics = hook.metrics();
        assert!(metrics.contains_key("retry.started"));
        assert!(metrics.contains_key("retry.success"));
    }

    // =========================================================================
    // TYPED EVENT TESTS
    // =========================================================================

    #[test]
    fn test_event_categories() {
        assert_eq!(RetryEventCategory::name(), "retry");
        assert_eq!(RetryEventCategory::default_log_level(), LogLevel::Info);
        assert!(!RetryEventCategory::is_sampled());

        assert_eq!(CircuitBreakerEventCategory::name(), "circuit_breaker");
        assert_eq!(
            CircuitBreakerEventCategory::default_log_level(),
            LogLevel::Warn
        );

        assert_eq!(TimeoutEventCategory::name(), "timeout");
        assert_eq!(TimeoutEventCategory::default_log_level(), LogLevel::Warn);

        assert_eq!(RateLimiterEventCategory::name(), "rate_limiter");
        assert!(RateLimiterEventCategory::is_sampled());

        assert_eq!(BulkheadEventCategory::name(), "bulkhead");
    }

    #[test]
    fn test_typed_event() {
        let event = Event::<RetryEventCategory>::new("api_call")
            .with_duration(Duration::from_millis(100))
            .with_context("attempt", "1")
            .with_context("max_attempts", "3");

        assert_eq!(event.operation, "api_call");
        assert_eq!(event.category(), "retry");
        assert_eq!(event.log_level(), LogLevel::Info);
        assert!(!event.is_sampled());
        assert!(!event.is_error());
        assert_eq!(event.duration, Some(Duration::from_millis(100)));
        assert_eq!(event.context.get("attempt"), Some(&"1".to_string()));
    }

    #[test]
    fn test_typed_event_with_error() {
        let event =
            Event::<CircuitBreakerEventCategory>::new("db_query").with_error("Connection timeout");

        assert!(event.is_error());
        assert_eq!(event.error, Some("Connection timeout".to_string()));
        assert_eq!(event.log_level(), LogLevel::Warn);
    }

    #[test]
    fn test_typed_metric() {
        let metric = Metric::<2>::new(
            "request_duration",
            125.5,
            [
                ("service", "api".to_string()),
                ("operation", "get_user".to_string()),
            ],
        );

        assert_eq!(metric.name, "request_duration");
        assert!((metric.value - 125.5).abs() < f64::EPSILON);
        assert_eq!(metric.labels[0], ("service", "api".to_string()));
        assert_eq!(metric.labels[1], ("operation", "get_user".to_string()));
    }

    #[test]
    fn test_metric_helpers() {
        let counter = metrics::counter("requests");
        assert_eq!(counter.name, "requests");
        assert!((counter.value - 1.0).abs() < f64::EPSILON);

        let gauge = metrics::service_gauge("connections", "db", 42.0);
        assert_eq!(gauge.name, "connections");
        assert_eq!(gauge.labels[1], ("service", "db".to_string()));

        let histogram = metrics::operation_histogram("latency", "api", "get_user", 125.0);
        assert_eq!(histogram.name, "latency");
        assert_eq!(histogram.labels[1], ("service", "api".to_string()));
        assert_eq!(histogram.labels[2], ("operation", "get_user".to_string()));
    }
}
