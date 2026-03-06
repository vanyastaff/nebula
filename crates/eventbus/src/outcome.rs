//! Publish outcomes for event emission APIs.

/// Result of publishing an event into the [`EventBus`](crate::EventBus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublishOutcome {
    /// Event was accepted by the bus and delivered to active subscribers.
    Sent,
    /// Event was dropped because there were no active subscribers.
    DroppedNoSubscribers,
    /// Event was dropped by a back-pressure policy decision.
    DroppedByPolicy,
    /// Event was dropped after timeout while waiting for free capacity.
    DroppedTimeout,
}

impl PublishOutcome {
    /// Returns `true` when the event was accepted by the bus.
    #[must_use]
    pub const fn is_sent(self) -> bool {
        matches!(self, Self::Sent)
    }
}
