//! EventSource topology (secondary) — pull-based event subscription.

use std::future::Future;

use crate::ctx::Ctx;
use crate::resource::Resource;

/// EventSource topology (secondary) — pull-based event subscription.
///
/// A secondary topology layered on top of a primary one. The runtime
/// produces events that callers consume via subscriptions.
pub trait EventSource: Resource {
    /// The event type produced by this source.
    type Event: Send + Clone + 'static;
    /// An opaque subscription handle for receiving events.
    type Subscription: Send + 'static;

    /// Creates a new subscription to this event source.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the subscription cannot be created.
    fn subscribe(
        &self,
        runtime: &Self::Runtime,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;

    /// Receives the next event from a subscription.
    ///
    /// This method blocks asynchronously until an event is available.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the subscription is broken or the source
    /// has been shut down.
    fn recv(
        &self,
        subscription: &mut Self::Subscription,
    ) -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}

/// Configuration types for event source topology.
pub mod config {
    /// EventSource configuration.
    #[derive(Debug, Clone, Default)]
    pub struct Config {
        /// Buffer size for the event channel.
        pub buffer_size: usize,
    }
}
