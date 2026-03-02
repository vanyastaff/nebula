//! Subscription handle for receiving events.

use tokio::sync::broadcast;

/// Subscription handle that receives events from an [`EventBus`](crate::EventBus).
///
/// Handles [`Lagged`](broadcast::error::RecvError::Lagged) by skipping to the latest
/// event so the subscriber does not block the producer.
#[derive(Debug)]
pub struct Subscriber<E> {
    receiver: broadcast::Receiver<E>,
}

impl<E: Clone + Send> Subscriber<E> {
    pub(crate) fn new(receiver: broadcast::Receiver<E>) -> Self {
        Self { receiver }
    }

    /// Receive the next event asynchronously.
    ///
    /// Returns `None` when the bus is closed (all senders dropped).
    /// On lag (buffer overflow), skips missed events and continues.
    pub async fn recv(&mut self) -> Option<E> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    }
}
