//! Subscription handle for receiving events.

use tokio::sync::broadcast;

/// Subscription handle that receives events from an [`EventBus`](crate::EventBus).
///
/// Handles [`Lagged`](broadcast::error::RecvError::Lagged) by skipping to the latest
/// event so the subscriber does not block the producer.
///
/// # Lifecycle
///
/// ## Drop Behavior
///
/// When a `Subscriber` is dropped, the underlying channel automatically decrements the subscriber count.
/// No explicit unsubscribe is needed — the subscription ends when the subscriber is dropped or
/// [`close()`](Self::close) is called.
///
/// **Note:** [`close()`](Self::close) is semantically equivalent to dropping the subscriber.
/// It exists for clarity when you want to explicitly signal subscription termination.
///
/// ## Lag Recovery
///
/// When the ring buffer fills (more events emitted than buffer size), subscribers
/// fall behind. Upon the next [`recv()`](Self::recv) or [`try_recv()`](Self::try_recv), the subscriber
/// automatically skips to the latest event and recovers without blocking the producer.
///
/// Use [`lagged_count()`](Self::lagged_count) to detect lag and monitor missed events:
///
/// ```no_run
/// # use nebula_eventbus::EventBus;
/// # #[derive(Clone)]
/// # struct Event(u64);
/// # #[tokio::main]
/// # async fn main() {
/// # let bus = EventBus::<Event>::new(10);
/// let mut sub = bus.subscribe();
/// // ... emit 20 events with a slow subscriber ...
/// // Later:
/// if let Some(event) = sub.recv().await {
///     let missed = sub.lagged_count();
///     if missed > 0 {
///         println!("Subscriber fell {} events behind", missed);
///     }
/// }
/// # }
/// ```
///
/// ## Closure Detection
///
/// [`is_closed()`](Self::is_closed) returns `true` only after all [`EventBus`](crate::EventBus)
/// senders are dropped. When the bus is closed, subsequent [`recv()`](Self::recv) calls return `None`.
#[derive(Debug)]
pub struct Subscriber<E> {
    receiver: broadcast::Receiver<E>,
    lagged_count: u64,
}

impl<E: Clone + Send> Subscriber<E> {
    pub(crate) fn new(receiver: broadcast::Receiver<E>) -> Self {
        Self {
            receiver,
            lagged_count: 0,
        }
    }

    /// Receive the next event asynchronously.
    ///
    /// Returns `None` when the bus is closed (all senders dropped).
    /// On lag (buffer overflow), skips missed events and continues.
    pub async fn recv(&mut self) -> Option<E> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    self.lagged_count = self.lagged_count.saturating_add(skipped);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Try to receive an event without blocking.
    ///
    /// Returns `None` if no event is available or the bus is closed.
    pub fn try_recv(&mut self) -> Option<E> {
        loop {
            match self.receiver.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    self.lagged_count = self.lagged_count.saturating_add(skipped);
                    continue;
                }
                Err(_) => return None,
            }
        }
    }

    /// Returns the total count of events skipped due to lag.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.lagged_count
    }

    /// Returns `true` if all senders are dropped.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.receiver.is_closed()
    }

    /// Closes this subscription handle by consuming it.
    pub fn close(self) {}
}
