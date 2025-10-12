//! Observability event and hook traits
//!
//! This module defines the core traits for the observability system:
//! - [`ObservabilityEvent`]: Events that can be emitted
//! - [`ObservabilityHook`]: Hooks that receive events

use std::time::SystemTime;

/// Event that can be emitted through observability system
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::ObservabilityEvent;
/// use std::time::SystemTime;
///
/// struct ValidationEvent {
///     field: String,
///     valid: bool,
/// }
///
/// impl ObservabilityEvent for ValidationEvent {
///     fn name(&self) -> &str {
///         "validation"
///     }
///
///     fn data(&self) -> Option<serde_json::Value> {
///         Some(serde_json::json!({
///             "field": self.field,
///             "valid": self.valid,
///         }))
///     }
/// }
/// ```
pub trait ObservabilityEvent: Send + Sync {
    /// Event name for identification
    ///
    /// Should be a stable identifier like "operation_started", "validation_failed", etc.
    fn name(&self) -> &str;

    /// When the event occurred
    ///
    /// Defaults to current time if not overridden.
    fn timestamp(&self) -> SystemTime {
        SystemTime::now()
    }

    /// Optional: serialize event data for structured logging
    ///
    /// Return `None` if the event has no additional data.
    fn data(&self) -> Option<serde_json::Value> {
        None
    }
}

/// Hook that receives observability events
///
/// Implement this trait to create custom hooks that respond to events.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{ObservabilityEvent, ObservabilityHook};
///
/// struct ConsoleHook;
///
/// impl ObservabilityHook for ConsoleHook {
///     fn on_event(&self, event: &dyn ObservabilityEvent) {
///         println!("[EVENT] {}", event.name());
///     }
///
///     fn initialize(&self) {
///         println!("ConsoleHook initialized");
///     }
///
///     fn shutdown(&self) {
///         println!("ConsoleHook shutdown");
///     }
/// }
/// ```
pub trait ObservabilityHook: Send + Sync {
    /// Called when an event occurs
    ///
    /// This method is called for every event emitted to the registry.
    /// Implementations should be fast and non-blocking.
    fn on_event(&self, event: &dyn ObservabilityEvent);

    /// Optional: initialize hook
    ///
    /// Called once when the hook is registered.
    fn initialize(&self) {}

    /// Optional: shutdown hook
    ///
    /// Called when the hook is being removed or during shutdown.
    fn shutdown(&self) {}
}

/// Built-in hook that logs events using tracing
///
/// This hook emits events as tracing log messages at the specified level.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{LoggingHook, register_hook};
/// use std::sync::Arc;
///
/// let hook = LoggingHook::new(tracing::Level::INFO);
/// register_hook(Arc::new(hook));
/// ```
pub struct LoggingHook {
    level: tracing::Level,
}

impl LoggingHook {
    /// Create a new logging hook with the specified log level
    pub fn new(level: tracing::Level) -> Self {
        Self { level }
    }
}

impl ObservabilityHook for LoggingHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        let event_name = event.name();
        let event_data = event.data();

        match self.level {
            tracing::Level::ERROR => {
                if let Some(data) = event_data {
                    tracing::error!(event = event_name, data = ?data, "observability event");
                } else {
                    tracing::error!(event = event_name, "observability event");
                }
            }
            tracing::Level::WARN => {
                if let Some(data) = event_data {
                    tracing::warn!(event = event_name, data = ?data, "observability event");
                } else {
                    tracing::warn!(event = event_name, "observability event");
                }
            }
            tracing::Level::INFO => {
                if let Some(data) = event_data {
                    tracing::info!(event = event_name, data = ?data, "observability event");
                } else {
                    tracing::info!(event = event_name, "observability event");
                }
            }
            tracing::Level::DEBUG => {
                if let Some(data) = event_data {
                    tracing::debug!(event = event_name, data = ?data, "observability event");
                } else {
                    tracing::debug!(event = event_name, "observability event");
                }
            }
            tracing::Level::TRACE => {
                if let Some(data) = event_data {
                    tracing::trace!(event = event_name, data = ?data, "observability event");
                } else {
                    tracing::trace!(event = event_name, "observability event");
                }
            }
        }
    }
}

/// Built-in hook that records events as metrics
///
/// This hook increments a counter for each event type.
/// Requires the `observability` feature flag.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_log::observability::{MetricsHook, register_hook};
/// use std::sync::Arc;
///
/// let hook = MetricsHook::new();
/// register_hook(Arc::new(hook));
/// ```
#[cfg(feature = "observability")]
pub struct MetricsHook;

#[cfg(feature = "observability")]
impl MetricsHook {
    /// Create a new metrics hook
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "observability")]
impl Default for MetricsHook {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "observability")]
impl ObservabilityHook for MetricsHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        // Increment counter for this event type
        let metric_name = format!("nebula.events.{}", event.name());
        crate::metrics::counter!(metric_name).increment(1);
    }
}

/// Resource-aware hook that can access node-scoped resources
///
/// This trait extends [`ObservabilityHook`] to provide access to the current
/// [`NodeContext`] and its resources. This allows hooks to access per-node
/// configuration like [`LoggerResource`].
///
/// # Security
///
/// Resources are scoped per-node and isolated. Hooks cannot access resources
/// from other nodes, ensuring multi-tenancy security.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{
///     ObservabilityEvent, ResourceAwareHook, NodeContext, LoggerResource
/// };
/// use std::sync::Arc;
///
/// struct NotificationHook;
///
/// impl ResourceAwareHook for NotificationHook {
///     fn on_event_with_context(&self, event: &dyn ObservabilityEvent, ctx: Option<Arc<NodeContext>>) {
///         if let Some(ctx) = ctx {
///             // Access LoggerResource from node context (if attached)
///             if let Some(logger) = ctx.get_resource::<LoggerResource>("LoggerResource") {
///                 if let Some(webhook) = logger.webhook_url() {
///                     // Send notification to webhook
///                     println!("Sending to webhook: {}", webhook);
///                 }
///             }
///         }
///     }
/// }
/// ```
pub trait ResourceAwareHook: Send + Sync {
    /// Called when an event occurs, with access to node context
    ///
    /// The `ctx` parameter contains the current [`NodeContext`] if available.
    /// Use [`NodeContext::get_resource`] to access node-scoped resources.
    fn on_event_with_context(
        &self,
        event: &dyn ObservabilityEvent,
        ctx: Option<std::sync::Arc<super::context::NodeContext>>,
    );

    /// Optional: initialize hook
    fn initialize(&self) {}

    /// Optional: shutdown hook
    fn shutdown(&self) {}
}

/// Adapter to use a [`ResourceAwareHook`] as an [`ObservabilityHook`]
///
/// This wrapper automatically fetches the current [`NodeContext`] and
/// passes it to the resource-aware hook.
pub struct ResourceAwareAdapter<H: ResourceAwareHook> {
    inner: H,
}

impl<H: ResourceAwareHook> ResourceAwareAdapter<H> {
    /// Create a new adapter for a resource-aware hook
    pub fn new(hook: H) -> Self {
        Self { inner: hook }
    }
}

impl<H: ResourceAwareHook> ObservabilityHook for ResourceAwareAdapter<H> {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        let ctx = super::context::NodeContext::current();
        self.inner.on_event_with_context(event, ctx);
    }

    fn initialize(&self) {
        self.inner.initialize();
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEvent {
        name: String,
    }

    impl ObservabilityEvent for TestEvent {
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_event_trait() {
        let event = TestEvent {
            name: "test_event".to_string(),
        };
        assert_eq!(event.name(), "test_event");
        assert!(event.data().is_none());
    }

    #[test]
    fn test_logging_hook() {
        let hook = LoggingHook::new(tracing::Level::INFO);
        let event = TestEvent {
            name: "test".to_string(),
        };
        // Should not panic
        hook.on_event(&event);
    }

    #[cfg(feature = "observability")]
    #[test]
    fn test_metrics_hook() {
        let hook = MetricsHook::new();
        let event = TestEvent {
            name: "test".to_string(),
        };
        // Should not panic
        hook.on_event(&event);
    }
}
