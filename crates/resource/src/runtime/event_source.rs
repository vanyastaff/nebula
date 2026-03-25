//! EventSource runtime — subscribe/recv wrapper for pull-based event sources.
//!
//! The event source runtime is a thin wrapper around the [`EventSource`] trait,
//! providing a consistent runtime API for creating subscriptions and receiving
//! events.

use std::marker::PhantomData;

use crate::ctx::Ctx;
use crate::error::Error;
use crate::resource::Resource;
use crate::topology::event_source::config::Config;
use crate::topology::event_source::EventSource;

/// Runtime state for an event source topology.
///
/// Unlike primary topologies (Pool, Resident, etc.), the event source runtime
/// does not manage instance lifecycle — it delegates to the underlying primary
/// runtime. Instead, it provides `subscribe` and `recv` operations.
pub struct EventSourceRuntime<R: Resource> {
    config: Config,
    _phantom: PhantomData<R>,
}

impl<R: Resource> EventSourceRuntime<R> {
    /// Creates a new event source runtime with the given configuration.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            _phantom: PhantomData,
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

impl<R> EventSourceRuntime<R>
where
    R: EventSource + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    /// Creates a new subscription to the event source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`EventSource::subscribe`].
    pub async fn subscribe(
        &self,
        resource: &R,
        runtime: &R::Runtime,
        ctx: &dyn Ctx,
    ) -> Result<R::Subscription, Error> {
        resource.subscribe(runtime, ctx).await.map_err(Into::into)
    }

    /// Receives the next event from a subscription.
    ///
    /// Blocks asynchronously until an event is available or the subscription
    /// is broken.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`EventSource::recv`].
    pub async fn recv(
        &self,
        resource: &R,
        subscription: &mut R::Subscription,
    ) -> Result<R::Event, Error> {
        resource.recv(subscription).await.map_err(Into::into)
    }
}
