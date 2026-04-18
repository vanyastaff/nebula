//! Stream adapters for event bus subscribers.

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use futures_core::Stream;
use tokio_stream::wrappers::BroadcastStream;

use crate::EventFilter;

/// Stream adapter that yields events from a [`Subscriber`](crate::Subscriber).
///
/// Created via [`Subscriber::into_stream()`](crate::Subscriber::into_stream).
/// Lagged events are skipped automatically (same semantics as
/// [`Subscriber::recv()`](crate::Subscriber::recv)).
///
/// # Example
///
/// ```no_run
/// use futures_core::Stream;
/// use nebula_eventbus::EventBus;
///
/// # #[derive(Clone)]
/// # struct Event(u64);
/// # async fn example() {
/// let bus = EventBus::<Event>::new(64);
/// let stream = bus.subscribe().into_stream();
/// // Use with StreamExt combinators
/// # }
/// ```
pub struct SubscriberStream<E: Clone + Send + 'static> {
    inner: BroadcastStream<E>,
    lagged_count: u64,
    bus_dropped: Arc<AtomicU64>,
}

impl<E: Clone + Send + 'static> SubscriberStream<E> {
    pub(crate) fn new(
        receiver: tokio::sync::broadcast::Receiver<E>,
        lagged_count: u64,
        bus_dropped: Arc<AtomicU64>,
    ) -> Self {
        Self {
            inner: BroadcastStream::new(receiver),
            lagged_count,
            bus_dropped,
        }
    }

    /// Returns the total count of events skipped due to lag.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.lagged_count
    }
}

impl<E: Clone + Send + 'static> Stream for SubscriberStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(event))) => return Poll::Ready(Some(event)),
                Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(skipped)))) => {
                    self.lagged_count = self.lagged_count.saturating_add(skipped);
                    self.bus_dropped.fetch_add(skipped, Ordering::Relaxed);
                    continue;
                },
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Stream adapter that filters events by predicate.
///
/// Created via [`FilteredSubscriber::into_stream()`](crate::FilteredSubscriber::into_stream).
pub struct FilteredStream<E: Clone + Send + 'static> {
    inner: SubscriberStream<E>,
    filter: EventFilter<E>,
}

impl<E: Clone + Send + 'static> FilteredStream<E> {
    pub(crate) fn new(inner: SubscriberStream<E>, filter: EventFilter<E>) -> Self {
        Self { inner, filter }
    }

    /// Returns the total count of events skipped due to lag.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.inner.lagged_count()
    }
}

impl<E: Clone + Send + 'static> Stream for FilteredStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(event)) => {
                    if self.filter.matches(&event) {
                        return Poll::Ready(Some(event));
                    }
                    continue;
                },
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
