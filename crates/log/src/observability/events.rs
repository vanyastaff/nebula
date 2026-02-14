//! Common observability event types
//!
//! This module provides pre-defined event types for common scenarios
//! like operation lifecycle tracking.

use super::hooks::ObservabilityEvent;
use std::time::Duration;

/// Event emitted when an operation starts
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{OperationStarted, emit_event};
///
/// let event = OperationStarted {
///     operation: "database_query".to_string(),
///     context: "user_fetch".to_string(),
/// };
/// emit_event(&event);
/// ```
#[derive(Debug, Clone)]
pub struct OperationStarted {
    /// Name of the operation
    pub operation: String,
    /// Additional context about the operation
    pub context: String,
}

impl ObservabilityEvent for OperationStarted {
    fn name(&self) -> &str {
        "operation_started"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "operation": self.operation,
            "context": self.context,
        }))
    }
}

/// Event emitted when an operation completes successfully
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{OperationCompleted, emit_event};
/// use std::time::Duration;
///
/// let event = OperationCompleted {
///     operation: "database_query".to_string(),
///     duration: Duration::from_millis(42),
/// };
/// emit_event(&event);
/// ```
#[derive(Debug, Clone)]
pub struct OperationCompleted {
    /// Name of the operation
    pub operation: String,
    /// How long the operation took
    pub duration: Duration,
}

impl ObservabilityEvent for OperationCompleted {
    fn name(&self) -> &str {
        "operation_completed"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "operation": self.operation,
            "duration_ms": self.duration.as_millis(),
            "duration_secs": self.duration.as_secs_f64(),
        }))
    }
}

/// Event emitted when an operation fails
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{OperationFailed, emit_event};
/// use std::time::Duration;
///
/// let event = OperationFailed {
///     operation: "database_query".to_string(),
///     error: "connection timeout".to_string(),
///     duration: Duration::from_millis(5000),
/// };
/// emit_event(&event);
/// ```
#[derive(Debug, Clone)]
pub struct OperationFailed {
    /// Name of the operation
    pub operation: String,
    /// Error message or description
    pub error: String,
    /// How long the operation ran before failing
    pub duration: Duration,
}

impl ObservabilityEvent for OperationFailed {
    fn name(&self) -> &str {
        "operation_failed"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "operation": self.operation,
            "error": self.error,
            "duration_ms": self.duration.as_millis(),
            "duration_secs": self.duration.as_secs_f64(),
        }))
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

        let event = OperationStarted {
            operation: operation.clone(),
            context,
        };
        super::emit_event(&event);

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
        let duration = self.start.elapsed();
        let event = OperationCompleted {
            operation: std::mem::take(&mut self.operation),
            duration,
        };
        super::emit_event(&event);
    }

    /// Mark the operation as failed with an error message
    ///
    /// Emits `OperationFailed` event.
    pub fn fail(mut self, error: impl Into<String>) {
        self.completed = true;
        let duration = self.start.elapsed();
        let event = OperationFailed {
            operation: std::mem::take(&mut self.operation),
            error: error.into(),
            duration,
        };
        super::emit_event(&event);
    }
}

impl Drop for OperationTracker {
    fn drop(&mut self) {
        if !self.completed {
            let duration = self.start.elapsed();
            let event = OperationFailed {
                operation: std::mem::take(&mut self.operation),
                error: "operation dropped without completion".to_string(),
                duration,
            };
            super::emit_event(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_started() {
        let event = OperationStarted {
            operation: "test".to_string(),
            context: "unit_test".to_string(),
        };
        assert_eq!(event.name(), "operation_started");
        assert!(event.data().is_some());
    }

    #[test]
    fn test_operation_completed() {
        let event = OperationCompleted {
            operation: "test".to_string(),
            duration: Duration::from_millis(100),
        };
        assert_eq!(event.name(), "operation_completed");
        let data = event.data().unwrap();
        assert_eq!(data["operation"], "test");
        assert_eq!(data["duration_ms"], 100);
    }

    #[test]
    fn test_operation_failed() {
        let event = OperationFailed {
            operation: "test".to_string(),
            error: "test error".to_string(),
            duration: Duration::from_millis(50),
        };
        assert_eq!(event.name(), "operation_failed");
        let data = event.data().unwrap();
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
