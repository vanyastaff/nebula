//! Observability hooks for pattern lifecycle events

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

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
}
