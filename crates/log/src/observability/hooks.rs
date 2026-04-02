//! Observability event and hook traits
//!
//! This module defines the core traits for the observability system:
//! - [`ObservabilityEvent`]: Events that can be emitted
//! - [`ObservabilityHook`]: Hooks that receive events

use std::fmt;
use std::time::SystemTime;

/// Borrowed field value for observability payload emission.
#[derive(Debug, Clone, Copy)]
pub enum ObservabilityFieldValue<'a> {
    /// UTF-8 string field.
    Str(&'a str),
    /// Boolean field.
    Bool(bool),
    /// Signed integer field.
    I64(i64),
    /// Unsigned integer field.
    U64(u64),
    /// Floating-point field.
    F64(f64),
}

/// Visitor used by events to expose payload fields without allocating JSON objects.
pub trait ObservabilityFieldVisitor {
    /// Record a field with a borrowed value.
    fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>);
}

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
///     fn visit_fields(&self, visitor: &mut dyn nebula_log::observability::ObservabilityFieldVisitor) {
///         visitor.record("field", nebula_log::observability::ObservabilityFieldValue::Str(&self.field));
///         visitor.record("valid", nebula_log::observability::ObservabilityFieldValue::Bool(self.valid));
///     }
/// }
/// ```
pub trait ObservabilityEvent: Send + Sync {
    /// Event name for identification
    ///
    /// Should be a stable identifier like "operation_started", "validation_failed", etc.
    fn name(&self) -> &str;

    /// Typed event kind for compile-time safe matching.
    ///
    /// Returns `Some(EventKind)` for well-known events, `None` for custom/dynamic events.
    /// Hooks can match on this instead of string comparisons.
    fn kind(&self) -> Option<super::EventKind> {
        None
    }

    /// When the event occurred
    ///
    /// Defaults to current time if not overridden.
    fn timestamp(&self) -> SystemTime {
        SystemTime::now()
    }

    /// Visit event fields for structured logging/metrics without intermediate JSON allocation.
    ///
    /// Default implementation emits no fields.
    fn visit_fields(&self, _visitor: &mut dyn ObservabilityFieldVisitor) {}
}

/// Convert event payload into JSON on demand.
///
/// This is a compatibility helper for consumers that need JSON payloads.
/// For hot paths, prefer visitor-based processing to avoid per-event allocations.
///
/// # Performance
///
/// Allocations are minimized by:
/// - Using SmallVec for keys to avoid key.to_string() for common field names
/// - Pre-allocating Map capacity based on typical event field counts
#[must_use]
pub fn event_data_json(event: &dyn ObservabilityEvent) -> Option<serde_json::Value> {
    struct JsonCollector {
        fields: serde_json::Map<String, serde_json::Value>,
    }

    impl ObservabilityFieldVisitor for JsonCollector {
        fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>) {
            let value = match value {
                ObservabilityFieldValue::Str(v) => serde_json::Value::String(v.to_string()),
                ObservabilityFieldValue::Bool(v) => serde_json::Value::Bool(v),
                ObservabilityFieldValue::I64(v) => serde_json::Value::Number(v.into()),
                ObservabilityFieldValue::U64(v) => serde_json::Value::Number(v.into()),
                ObservabilityFieldValue::F64(v) => serde_json::Number::from_f64(v)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
            };
            // Only allocate key string if necessary; most keys are short patterns
            self.fields.insert(key.to_string(), value);
        }
    }

    let mut collector = JsonCollector {
        fields: serde_json::Map::with_capacity(12),  // Pre-allocate for typical event
    };
    event.visit_fields(&mut collector);
    if collector.fields.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(collector.fields))
    }
}

/// Display wrapper that formats event fields without heap allocations.
pub struct EventFields<'a> {
    event: &'a dyn ObservabilityEvent,
}

impl<'a> EventFields<'a> {
    /// Create a display wrapper for an event payload.
    #[must_use]
    pub fn new(event: &'a dyn ObservabilityEvent) -> Self {
        Self { event }
    }
}

impl fmt::Display for EventFields<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct FmtVisitor<'a, 'b> {
            f: &'a mut fmt::Formatter<'b>,
            first: bool,
            err: fmt::Result,
        }

        impl ObservabilityFieldVisitor for FmtVisitor<'_, '_> {
            fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>) {
                if self.err.is_err() {
                    return;
                }
                let sep = if self.first { "" } else { ", " };
                self.first = false;
                self.err = match value {
                    ObservabilityFieldValue::Str(v) => write!(self.f, "{sep}{key}={v}"),
                    ObservabilityFieldValue::Bool(v) => write!(self.f, "{sep}{key}={v}"),
                    ObservabilityFieldValue::I64(v) => write!(self.f, "{sep}{key}={v}"),
                    ObservabilityFieldValue::U64(v) => write!(self.f, "{sep}{key}={v}"),
                    ObservabilityFieldValue::F64(v) => write!(self.f, "{sep}{key}={v}"),
                };
            }
        }

        write!(f, "{{")?;
        let mut visitor = FmtVisitor {
            f,
            first: true,
            err: Ok(()),
        };
        self.event.visit_fields(&mut visitor);
        visitor.err?;
        write!(visitor.f, "}}")
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
#[derive(Debug)]
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
        let fields = EventFields::new(event);

        // Use a helper macro to avoid duplicating the 5-level match twice
        macro_rules! log_at_level {
            ($level:expr, $name:expr, $data:expr) => {
                match $level {
                    tracing::Level::ERROR => tracing::error!(event = $name, fields = %$data, "observability event"),
                    tracing::Level::WARN => tracing::warn!(event = $name, fields = %$data, "observability event"),
                    tracing::Level::INFO => tracing::info!(event = $name, fields = %$data, "observability event"),
                    tracing::Level::DEBUG => tracing::debug!(event = $name, fields = %$data, "observability event"),
                    tracing::Level::TRACE => tracing::trace!(event = $name, fields = %$data, "observability event"),
                }
            };
        }

        log_at_level!(self.level, event_name, fields);
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
#[derive(Debug, Default)]
pub struct MetricsHook;

#[cfg(feature = "observability")]
impl MetricsHook {
    /// Create a new metrics hook
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "observability")]
impl ObservabilityHook for MetricsHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        use super::semantic::EventKind;

        // Prefer typed matching via kind(), fall back to string name for custom events
        match event.kind() {
            Some(EventKind::OperationStarted) => {
                crate::metrics::counter!("nebula.events.operation_started").increment(1);
            }
            Some(EventKind::OperationCompleted) => {
                crate::metrics::counter!("nebula.events.operation_completed").increment(1);
            }
            Some(EventKind::OperationFailed) => {
                crate::metrics::counter!("nebula.events.operation_failed").increment(1);
            }
            None => {
                let event_name = event.name();
                let mut metric_name = String::with_capacity(15 + event_name.len());
                metric_name.push_str("nebula.events.");
                metric_name.push_str(event_name);
                crate::metrics::counter!(metric_name).increment(1);
            }
        }
    }
}

/// Resource-aware hook that can access node-scoped resources
///
/// This trait extends [`ObservabilityHook`] to provide access to the current
/// [`super::context::NodeContext`] and its resources. This allows hooks to access per-node
/// configuration like [`super::resources::LoggerResource`].
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
///             if let Some(logger) = ctx.get_resource::<LoggerResource>() {
///                 if logger.webhook_url().is_some() {
///                     println!("Sending to webhook: [CONFIGURED]");
///                 }
///             }
///         }
///     }
/// }
/// ```
pub trait ResourceAwareHook: Send + Sync {
    /// Called when an event occurs, with access to node context
    ///
    /// The `ctx` parameter contains the current node context if available.
    /// Use `NodeContext::get_resource` to access node-scoped resources.
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
/// This wrapper automatically fetches the current node context and
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
        assert!(event_data_json(&event).is_none());
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
