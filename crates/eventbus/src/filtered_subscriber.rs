//! Filtered subscriber wrapper.

use crate::EventFilter;
use crate::Subscriber;

/// Subscriber wrapper that yields only events matching a filter.
#[derive(Debug)]
pub struct FilteredSubscriber<E> {
    inner: Subscriber<E>,
    filter: EventFilter<E>,
}

impl<E: Clone + Send> FilteredSubscriber<E> {
    pub(crate) fn new(inner: Subscriber<E>, filter: EventFilter<E>) -> Self {
        Self { inner, filter }
    }

    /// Receives the next matching event asynchronously.
    ///
    /// Returns `None` when the underlying bus is closed.
    pub async fn recv(&mut self) -> Option<E> {
        loop {
            let event = self.inner.recv().await?;
            if self.filter.matches(&event) {
                return Some(event);
            }
        }
    }

    /// Tries to receive the next matching event without blocking.
    #[must_use]
    pub fn try_recv(&mut self) -> Option<E> {
        loop {
            let event = self.inner.try_recv()?;
            if self.filter.matches(&event) {
                return Some(event);
            }
        }
    }

    /// Returns the number of lagged events seen by the underlying subscriber.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.inner.lagged_count()
    }

    /// Returns `true` if the underlying channel is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Closes this subscription handle.
    pub fn close(self) {
        self.inner.close();
    }
}
