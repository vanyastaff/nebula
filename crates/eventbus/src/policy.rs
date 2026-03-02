//! Back-pressure policy when the event buffer is full.

use std::time::Duration;

/// Policy controlling what happens when the event buffer is full.
///
/// Default is [`DropOldest`](Self::DropOldest): emit path is non-blocking;
/// lagging subscribers receive fewer events (best-effort).
#[derive(Debug, Clone, Default)]
pub enum BackPressurePolicy {
    /// Overwrite the oldest unread event for lagging subscribers.
    ///
    /// Matches `tokio::sync::broadcast` behaviour. Emit never blocks.
    #[default]
    DropOldest,

    /// Discard the new event when the buffer is at capacity.
    ///
    /// Emitter is never blocked; the newest event is not sent.
    DropNewest,

    /// Block the emitter for up to `timeout` waiting for buffer space.
    ///
    /// Use [`EventBus::send_async`](crate::EventBus::send_async) for this policy.
    /// Synchronous [`EventBus::send`](crate::EventBus::send) falls back to
    /// `DropOldest` when this policy is set.
    Block {
        /// Maximum time to wait before dropping the event.
        timeout: Duration,
    },
}
