//! Scope primitives for scoped subscriptions.

/// Logical scope used to filter events for targeted subscribers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubscriptionScope {
    /// Receive all events regardless of IDs.
    Global,
    /// Receive events associated with a workflow ID.
    Workflow(String),
    /// Receive events associated with an execution ID.
    Execution(String),
    /// Receive events associated with a resource ID/key.
    Resource(String),
}

impl SubscriptionScope {
    /// Constructs a workflow scope.
    #[must_use]
    pub fn workflow(id: impl Into<String>) -> Self {
        Self::Workflow(id.into())
    }

    /// Constructs an execution scope.
    #[must_use]
    pub fn execution(id: impl Into<String>) -> Self {
        Self::Execution(id.into())
    }

    /// Constructs a resource scope.
    #[must_use]
    pub fn resource(id: impl Into<String>) -> Self {
        Self::Resource(id.into())
    }
}

/// Metadata extraction trait for scope-aware filtering.
///
/// Event types can implement this trait to enable
/// [`crate::EventBus::subscribe_scoped`].
pub trait ScopedEvent {
    /// Returns a workflow ID if the event is associated with one.
    fn workflow_id(&self) -> Option<&str> {
        None
    }

    /// Returns an execution ID if the event is associated with one.
    fn execution_id(&self) -> Option<&str> {
        None
    }

    /// Returns a resource ID if the event is associated with one.
    fn resource_id(&self) -> Option<&str> {
        None
    }

    /// Returns `true` if the event belongs to the provided scope.
    fn matches_scope(&self, scope: &SubscriptionScope) -> bool {
        match scope {
            SubscriptionScope::Global => true,
            SubscriptionScope::Workflow(id) => self.workflow_id() == Some(id.as_str()),
            SubscriptionScope::Execution(id) => self.execution_id() == Some(id.as_str()),
            SubscriptionScope::Resource(id) => self.resource_id() == Some(id.as_str()),
        }
    }
}
