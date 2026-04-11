//! Common observability event types
//!
//! This module provides pre-defined event types for common scenarios
//! like operation lifecycle tracking.

use std::{borrow::Cow, time::Duration};

use super::{
    hooks::{ObservabilityEvent, ObservabilityFieldValue, ObservabilityFieldVisitor},
    semantic::{EventKind, field},
};

/// Convert `Duration` milliseconds to `u64` with saturation instead of truncation.
fn saturating_millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Event emitted when an operation starts
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{OperationStarted, emit_event};
///
/// emit_event(&OperationStarted::new("database_query", "user_fetch"));
/// ```
#[derive(Debug, Clone)]
pub struct OperationStarted {
    /// Name of the operation
    pub operation: String,
    /// Additional context about the operation
    pub context: String,
}

impl OperationStarted {
    /// Create a new operation-started event.
    pub fn new(operation: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            context: context.into(),
        }
    }
}

impl ObservabilityEvent for OperationStarted {
    fn name(&self) -> &str {
        EventKind::OperationStarted.as_str()
    }

    fn kind(&self) -> Option<EventKind> {
        Some(EventKind::OperationStarted)
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record(
            field::OPERATION,
            ObservabilityFieldValue::Str(&self.operation),
        );
        visitor.record(field::CONTEXT, ObservabilityFieldValue::Str(&self.context));
    }
}

/// Event emitted when an operation completes successfully
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_log::observability::{OperationCompleted, emit_event};
///
/// emit_event(&OperationCompleted::new(
///     "database_query",
///     Duration::from_millis(42),
/// ));
/// ```
#[derive(Debug, Clone)]
pub struct OperationCompleted {
    /// Name of the operation
    pub operation: String,
    /// How long the operation took
    pub duration: Duration,
}

impl OperationCompleted {
    /// Create a new operation-completed event.
    pub fn new(operation: impl Into<String>, duration: Duration) -> Self {
        Self {
            operation: operation.into(),
            duration,
        }
    }
}

impl ObservabilityEvent for OperationCompleted {
    fn name(&self) -> &str {
        EventKind::OperationCompleted.as_str()
    }

    fn kind(&self) -> Option<EventKind> {
        Some(EventKind::OperationCompleted)
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record(
            field::OPERATION,
            ObservabilityFieldValue::Str(&self.operation),
        );
        visitor.record(
            field::DURATION_MS,
            ObservabilityFieldValue::U64(saturating_millis(self.duration)),
        );
        visitor.record(
            field::DURATION_SECS,
            ObservabilityFieldValue::F64(self.duration.as_secs_f64()),
        );
    }
}

/// Event emitted when an operation fails
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_log::observability::{OperationFailed, emit_event};
///
/// emit_event(&OperationFailed::new(
///     "database_query",
///     "connection timeout",
///     Duration::from_millis(5000),
/// ));
/// ```
#[derive(Debug, Clone)]
pub struct OperationFailed {
    /// Name of the operation
    pub operation: String,
    /// Error message or description.
    /// `Cow` avoids heap allocation for static messages (e.g. drop path).
    pub error: Cow<'static, str>,
    /// How long the operation ran before failing
    pub duration: Duration,
}

impl OperationFailed {
    /// Create a new operation-failed event.
    pub fn new(
        operation: impl Into<String>,
        error: impl Into<Cow<'static, str>>,
        duration: Duration,
    ) -> Self {
        Self {
            operation: operation.into(),
            error: error.into(),
            duration,
        }
    }
}

impl ObservabilityEvent for OperationFailed {
    fn name(&self) -> &str {
        EventKind::OperationFailed.as_str()
    }

    fn kind(&self) -> Option<EventKind> {
        Some(EventKind::OperationFailed)
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        visitor.record(
            field::OPERATION,
            ObservabilityFieldValue::Str(&self.operation),
        );
        visitor.record(field::ERROR, ObservabilityFieldValue::Str(&self.error));
        visitor.record(
            field::DURATION_MS,
            ObservabilityFieldValue::U64(saturating_millis(self.duration)),
        );
        visitor.record(
            field::DURATION_SECS,
            ObservabilityFieldValue::F64(self.duration.as_secs_f64()),
        );
    }
}

/// Helper to track operation lifecycle automatically
///
/// This struct uses RAII to automatically emit `OperationStarted` on creation
/// and either `OperationCompleted` or `OperationFailed` on drop.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::OperationTracker;
///
/// {
///     let tracker = OperationTracker::new("my_operation", "user_context");
///
///     // Do work...
///
///     tracker.success(); // Emits OperationCompleted
/// } // If success() not called, emits OperationFailed on drop
/// ```
#[derive(Debug)]
pub struct OperationTracker {
    operation: String,
    start: std::time::Instant,
    completed: bool,
}

impl OperationTracker {
    /// Create a new operation tracker and emit `OperationStarted`
    pub fn new(operation: impl Into<String>, context: impl Into<String>) -> Self {
        let operation = operation.into();
        let context = context.into();

        super::emit_event(&OperationStarted {
            operation: operation.clone(),
            context,
        });

        Self {
            operation,
            start: std::time::Instant::now(),
            completed: false,
        }
    }

    /// Mark the operation as successful
    ///
    /// Emits `OperationCompleted` event.
    pub fn success(mut self) {
        self.completed = true;
        super::emit_event(&OperationCompleted::new(
            std::mem::take(&mut self.operation),
            self.start.elapsed(),
        ));
    }

    /// Mark the operation as failed with an error message
    ///
    /// Emits `OperationFailed` event.
    pub fn fail(mut self, error: impl Into<Cow<'static, str>>) {
        self.completed = true;
        super::emit_event(&OperationFailed::new(
            std::mem::take(&mut self.operation),
            error,
            self.start.elapsed(),
        ));
    }
}

impl Drop for OperationTracker {
    fn drop(&mut self) {
        if !self.completed {
            super::emit_event(&OperationFailed::new(
                std::mem::take(&mut self.operation),
                "operation dropped without completion",
                self.start.elapsed(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::event_data_json;

    #[test]
    fn test_operation_started() {
        let event = OperationStarted::new("test", "unit_test");
        assert_eq!(event.name(), "operation_started");
        assert!(event_data_json(&event).is_some());
    }

    #[test]
    fn test_operation_completed() {
        let event = OperationCompleted::new("test", Duration::from_millis(100));
        assert_eq!(event.name(), "operation_completed");
        let data = event_data_json(&event).unwrap();
        assert_eq!(data["operation"], "test");
        assert_eq!(data["duration_ms"], 100);
    }

    #[test]
    fn test_operation_failed() {
        let event = OperationFailed::new("test", "test error", Duration::from_millis(50));
        assert_eq!(event.name(), "operation_failed");
        let data = event_data_json(&event).unwrap();
        assert_eq!(data["operation"], "test");
        assert_eq!(data["error"], "test error");
    }

    #[test]
    fn test_operation_tracker_success() {
        let tracker = OperationTracker::new("test_op", "test_ctx");
        tracker.success();
        // Should emit OperationStarted and OperationCompleted
    }

    #[test]
    fn test_operation_tracker_fail() {
        let tracker = OperationTracker::new("test_op", "test_ctx");
        tracker.fail("test error");
        // Should emit OperationStarted and OperationFailed
    }

    #[test]
    fn test_operation_tracker_drop() {
        {
            let _tracker = OperationTracker::new("test_op", "test_ctx");
            // Will drop without calling success() or fail()
        }
        // Should emit OperationStarted and OperationFailed (due to drop)
    }
}
