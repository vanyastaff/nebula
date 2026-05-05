//! `MetricsSink` — event sink for resilience observability.
//!
//! Replaces the custom `ObservabilityHook` system. The default is [`NoopSink`].
//! In nebula-engine, `EventBusSink` wraps nebula-eventbus — no direct dep here.

use std::{borrow::Cow, ops::Deref, sync::Arc, time::Duration};

use parking_lot::Mutex;

use crate::CallErrorKind;

/// Low-cardinality scope string shared by [`PolicyScope`].
///
/// Scope values are copied into an `Arc<str>` once at construction time. Cloning
/// a [`PolicyScope`] or [`ResilienceEvent::PipelineCompleted`] then increments a
/// refcount instead of allocating and copying tenant/workflow/action strings on
/// every pipeline completion event.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ScopeValue(Arc<str>);

impl ScopeValue {
    /// Borrow the scope value as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for ScopeValue {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for ScopeValue {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for ScopeValue {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for ScopeValue {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl From<Box<str>> for ScopeValue {
    fn from(value: Box<str>) -> Self {
        Self(Arc::from(value))
    }
}

impl From<Arc<str>> for ScopeValue {
    fn from(value: Arc<str>) -> Self {
        Self(value)
    }
}

impl<'a> From<Cow<'a, str>> for ScopeValue {
    fn from(value: Cow<'a, str>) -> Self {
        match value {
            Cow::Borrowed(value) => Self::from(value),
            Cow::Owned(value) => Self::from(value),
        }
    }
}

/// Workflow/resource scope attached to high-level pipeline events.
///
/// Scope values are intentionally optional so callers can choose a low-cardinality
/// subset that is safe for their telemetry backend. Dynamic values should usually
/// go to traces/events, not metric labels.
///
/// Field values use [`ScopeValue`] so pipeline completion events can carry scope
/// without deep-copying owned strings on every event clone.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PolicyScope {
    /// Tenant identifier, when safe to expose.
    pub tenant_id: Option<ScopeValue>,
    /// Workflow identifier or workflow type.
    pub workflow_id: Option<ScopeValue>,
    /// Action identifier or action type.
    pub action_id: Option<ScopeValue>,
    /// Resource identifier or resource type.
    pub resource_id: Option<ScopeValue>,
    /// Operation name, such as `gmail.poll` or `postgres.query`.
    pub operation: Option<ScopeValue>,
}

impl PolicyScope {
    /// Empty scope.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            tenant_id: None,
            workflow_id: None,
            action_id: None,
            resource_id: None,
            operation: None,
        }
    }

    /// Whether every scope field is unset.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tenant_id.is_none()
            && self.workflow_id.is_none()
            && self.action_id.is_none()
            && self.resource_id.is_none()
            && self.operation.is_none()
    }

    /// Set the tenant id.
    #[must_use]
    pub fn tenant_id(mut self, tenant_id: impl Into<ScopeValue>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Set the workflow id.
    #[must_use]
    pub fn workflow_id(mut self, workflow_id: impl Into<ScopeValue>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    /// Set the action id.
    #[must_use]
    pub fn action_id(mut self, action_id: impl Into<ScopeValue>) -> Self {
        self.action_id = Some(action_id.into());
        self
    }

    /// Set the resource id.
    #[must_use]
    pub fn resource_id(mut self, resource_id: impl Into<ScopeValue>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Set the operation name.
    #[must_use]
    pub fn operation(mut self, operation: impl Into<ScopeValue>) -> Self {
        self.operation = Some(operation.into());
        self
    }
}

/// Final outcome of a pipeline invocation.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PipelineOutcome {
    /// Pipeline returned the primary operation result.
    Success,
    /// Pipeline failed and no fallback recovered it.
    Failure {
        /// Final failure kind.
        error: CallErrorKind,
    },
    /// Fallback recovered the primary failure.
    FallbackSucceeded {
        /// Primary failure kind that was recovered.
        primary_error: CallErrorKind,
    },
    /// Fallback was attempted but failed.
    FallbackFailed {
        /// Primary failure kind that triggered fallback.
        primary_error: CallErrorKind,
        /// Fallback failure kind.
        fallback_error: CallErrorKind,
    },
}

/// A state in the circuit breaker state machine.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Breaker tripped — requests rejected immediately.
    Open,
    /// Probing — limited requests allowed to test recovery.
    HalfOpen,
}

/// Events emitted by resilience patterns to the [`MetricsSink`].
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResilienceEvent {
    /// Circuit breaker transitioned between states.
    CircuitStateChanged {
        /// Previous circuit state.
        from: CircuitState,
        /// New circuit state.
        to: CircuitState,
    },
    /// A retry attempt was made.
    RetryAttempt {
        /// 1-based attempt number.
        attempt: u32,
        /// Whether another attempt will follow.
        will_retry: bool,
    },
    /// A bulkhead rejected a request (at capacity).
    BulkheadRejected,
    /// A timeout elapsed.
    TimeoutElapsed {
        /// Configured timeout duration.
        duration: Duration,
    },
    /// A hedge request was fired.
    HedgeFired {
        /// 1-based hedge request number.
        hedge_number: u32,
    },
    /// A rate limit was exceeded.
    RateLimitExceeded,
    /// Load shed — request rejected due to overload.
    LoadShed,
    /// Fallback was selected for a failed primary operation.
    FallbackAttempted {
        /// Primary failure kind that triggered fallback consideration.
        primary_error: CallErrorKind,
    },
    /// Fallback returned a recovered value.
    FallbackSucceeded {
        /// Primary failure kind that was recovered by fallback.
        primary_error: CallErrorKind,
    },
    /// Fallback was attempted but returned an error.
    FallbackFailed {
        /// Primary failure kind that fallback failed to recover.
        primary_error: CallErrorKind,
        /// Fallback failure kind.
        fallback_error: CallErrorKind,
    },
    /// A pipeline invocation completed.
    PipelineCompleted {
        /// Caller-provided workflow/resource scope.
        scope: PolicyScope,
        /// Final pipeline outcome, including fallback recovery if used.
        outcome: PipelineOutcome,
    },
}

/// Fieldless discriminant of [`ResilienceEvent`] for type-safe event filtering.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResilienceEventKind {
    /// [`ResilienceEvent::CircuitStateChanged`]
    CircuitStateChanged,
    /// [`ResilienceEvent::RetryAttempt`]
    RetryAttempt,
    /// [`ResilienceEvent::BulkheadRejected`]
    BulkheadRejected,
    /// [`ResilienceEvent::TimeoutElapsed`]
    TimeoutElapsed,
    /// [`ResilienceEvent::HedgeFired`]
    HedgeFired,
    /// [`ResilienceEvent::RateLimitExceeded`]
    RateLimitExceeded,
    /// [`ResilienceEvent::LoadShed`]
    LoadShed,
    /// [`ResilienceEvent::FallbackAttempted`]
    FallbackAttempted,
    /// [`ResilienceEvent::FallbackSucceeded`]
    FallbackSucceeded,
    /// [`ResilienceEvent::FallbackFailed`]
    FallbackFailed,
    /// [`ResilienceEvent::PipelineCompleted`]
    PipelineCompleted,
}

/// Receives resilience events for observability (metrics, logging, `EventBus`).
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// use nebula_resilience::{MetricsSink, ResilienceEvent};
///
/// #[derive(Default)]
/// struct CountingSink {
///     calls: AtomicUsize,
/// }
///
/// impl MetricsSink for CountingSink {
///     fn record(&self, _event: ResilienceEvent) {
///         self.calls.fetch_add(1, Ordering::Relaxed);
///     }
/// }
///
/// let sink = CountingSink::default();
/// sink.record(ResilienceEvent::LoadShed);
/// assert_eq!(sink.calls.load(Ordering::Relaxed), 1);
/// ```
pub trait MetricsSink: Send + Sync {
    /// Record a resilience event.
    fn record(&self, event: ResilienceEvent);
}

/// Default sink — discards all events. Zero cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSink;

impl MetricsSink for NoopSink {
    fn record(&self, _: ResilienceEvent) {}
}

/// Test sink — records all events for assertion.
///
/// Cheap to clone; all clones share the same backing buffer, so a sink handed
/// to a pattern under test can be inspected from the test thread.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{MetricsSink, RecordingSink, ResilienceEvent, ResilienceEventKind};
///
/// let sink = RecordingSink::new();
/// sink.record(ResilienceEvent::BulkheadRejected);
/// sink.record(ResilienceEvent::LoadShed);
///
/// assert_eq!(sink.count(ResilienceEventKind::BulkheadRejected), 1);
/// assert_eq!(sink.count(ResilienceEventKind::LoadShed), 1);
/// ```
#[derive(Debug, Default, Clone)]
pub struct RecordingSink {
    events: Arc<Mutex<Vec<ResilienceEvent>>>,
}

impl RecordingSink {
    /// Create a new empty recording sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all recorded events.
    #[must_use]
    pub fn events(&self) -> Vec<ResilienceEvent> {
        self.events.lock().clone()
    }

    /// Count events matching a given kind.
    #[must_use]
    pub fn count(&self, kind: ResilienceEventKind) -> usize {
        self.events
            .lock()
            .iter()
            .filter(|e| e.kind() == kind)
            .count()
    }

    /// Returns true if a `CircuitStateChanged` event to `to` was recorded.
    #[must_use]
    pub fn has_state_change(&self, to: CircuitState) -> bool {
        self.events()
            .iter()
            .any(|e| matches!(e, ResilienceEvent::CircuitStateChanged { to: t, .. } if *t == to))
    }
}

impl MetricsSink for RecordingSink {
    fn record(&self, event: ResilienceEvent) {
        self.events.lock().push(event);
    }
}

impl ResilienceEvent {
    /// Returns the fieldless discriminant of this event.
    #[must_use]
    pub const fn kind(&self) -> ResilienceEventKind {
        match self {
            Self::CircuitStateChanged { .. } => ResilienceEventKind::CircuitStateChanged,
            Self::RetryAttempt { .. } => ResilienceEventKind::RetryAttempt,
            Self::BulkheadRejected => ResilienceEventKind::BulkheadRejected,
            Self::TimeoutElapsed { .. } => ResilienceEventKind::TimeoutElapsed,
            Self::HedgeFired { .. } => ResilienceEventKind::HedgeFired,
            Self::RateLimitExceeded => ResilienceEventKind::RateLimitExceeded,
            Self::LoadShed => ResilienceEventKind::LoadShed,
            Self::FallbackAttempted { .. } => ResilienceEventKind::FallbackAttempted,
            Self::FallbackSucceeded { .. } => ResilienceEventKind::FallbackSucceeded,
            Self::FallbackFailed { .. } => ResilienceEventKind::FallbackFailed,
            Self::PipelineCompleted { .. } => ResilienceEventKind::PipelineCompleted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_captures_events() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::BulkheadRejected);
        sink.record(ResilienceEvent::BulkheadRejected);
        assert_eq!(sink.count(ResilienceEventKind::BulkheadRejected), 2);
    }

    #[test]
    fn recording_sink_detects_state_change() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::CircuitStateChanged {
            from: CircuitState::Closed,
            to: CircuitState::Open,
        });
        assert!(sink.has_state_change(CircuitState::Open));
        assert!(!sink.has_state_change(CircuitState::HalfOpen));
    }

    #[test]
    fn noop_sink_does_not_panic() {
        let sink = NoopSink;
        sink.record(ResilienceEvent::LoadShed); // just must not panic
    }

    #[test]
    fn policy_scope_builders_set_fields() {
        let tenant = String::from("tenant-a");
        let resource = Cow::Borrowed("resource-a");
        let scope = PolicyScope::empty()
            .tenant_id(tenant)
            .workflow_id("workflow-a")
            .action_id("action-a")
            .resource_id(resource)
            .operation("gmail.poll");

        assert_eq!(scope.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(scope.resource_id.as_deref(), Some("resource-a"));
        assert_eq!(scope.operation.as_deref(), Some("gmail.poll"));
    }

    #[test]
    fn policy_scope_clone_shares_owned_values() {
        let tenant = Arc::<str>::from("tenant-a");
        let scope = PolicyScope::empty().tenant_id(Arc::clone(&tenant));
        let cloned = scope.clone();

        let original = scope.tenant_id.as_ref().unwrap();
        let cloned = cloned.tenant_id.as_ref().unwrap();

        assert_eq!(original.as_str(), "tenant-a");
        assert!(Arc::ptr_eq(&tenant, &original.0));
        assert!(Arc::ptr_eq(&original.0, &cloned.0));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn scoped_pipeline_event_serde_round_trips() {
        let event = ResilienceEvent::PipelineCompleted {
            scope: PolicyScope::empty()
                .tenant_id("tenant-a")
                .workflow_id("workflow-a")
                .operation("gmail.poll"),
            outcome: PipelineOutcome::FallbackSucceeded {
                primary_error: CallErrorKind::Timeout,
            },
        };

        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: ResilienceEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, event);
    }
}
