//! Event filtering primitives for filtered subscriptions.

use std::sync::Arc;

use crate::scope::{ScopedEvent, SubscriptionScope};

/// Predicate wrapper used by [`crate::FilteredSubscriber`].
#[derive(Clone)]
pub struct EventFilter<E> {
    predicate: Arc<dyn Fn(&E) -> bool + Send + Sync + 'static>,
}

impl<E> std::fmt::Debug for EventFilter<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFilter").finish_non_exhaustive()
    }
}

impl<E> EventFilter<E> {
    /// Creates a filter that accepts all events.
    #[must_use]
    pub fn all() -> Self {
        Self::custom(|_| true)
    }

    /// Creates a custom predicate-based filter.
    #[must_use]
    pub fn custom(predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        Self {
            predicate: Arc::new(predicate),
        }
    }

    /// Creates a scope-based filter.
    #[must_use]
    pub fn by_scope(scope: SubscriptionScope) -> Self
    where
        E: ScopedEvent,
    {
        Self::custom(move |event| event.matches_scope(&scope))
    }

    /// Returns `true` when the event passes this filter.
    #[must_use]
    pub fn matches(&self, event: &E) -> bool {
        (self.predicate)(event)
    }
}

impl<E> Default for EventFilter<E> {
    fn default() -> Self {
        Self::all()
    }
}
