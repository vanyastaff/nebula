//! Bounded stream buffer with explicit backpressure policies.
//!
//! This is a runtime-level primitive for stream-oriented action outputs where
//! producer and consumer rates may diverge.

use std::{collections::VecDeque, sync::Arc};

use tokio::sync::{Mutex, MutexGuard, Notify};

use crate::RuntimeError;

/// Overflow policy for a bounded stream buffer when it is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Block the producer until buffer space is available.
    Block,
    /// Evict the oldest buffered item to make room.
    DropOldest,
    /// Drop the incoming item silently.
    DropNewest,
    /// Return an error to the producer.
    Error,
}

/// Result of pushing an item into a bounded stream buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PushOutcome {
    /// Item was accepted without eviction.
    Accepted,
    /// Item was accepted after evicting one buffered item.
    AcceptedAfterDropOldest,
    /// Item was dropped due to `Overflow::DropNewest`.
    DroppedNewest,
}

impl PushOutcome {
    /// Upgrade the outcome to indicate that an old item was evicted to make room.
    const fn with_dropped_oldest(self) -> Self {
        match self {
            PushOutcome::Accepted => PushOutcome::AcceptedAfterDropOldest,
            other => other,
        }
    }
}

#[derive(Debug)]
struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    not_empty: Notify,
    not_full: Notify,
    capacity: usize,
    overflow: Overflow,
}

/// Async bounded queue used for streaming backpressure tests and runtime flow.
#[derive(Debug, Clone)]
pub struct BoundedStreamBuffer<T> {
    inner: Arc<Inner<T>>,
}

impl<T> BoundedStreamBuffer<T> {
    /// Create a bounded stream buffer.
    #[must_use]
    pub fn new(capacity: usize, overflow: Overflow) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        Self {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::with_capacity(capacity)),
                not_empty: Notify::new(),
                not_full: Notify::new(),
                capacity,
                overflow,
            }),
        }
    }

    /// Push an item according to configured overflow policy.
    pub async fn push(&self, item: T) -> Result<PushOutcome, RuntimeError> {
        loop {
            let mut queue = self.inner.queue.lock().await;

            if queue.len() < self.inner.capacity {
                return Ok(self.accept(queue, item));
            }

            match self.inner.overflow {
                Overflow::Block => self.wait_for_space(queue).await,
                Overflow::DropOldest => {
                    let _ = queue.pop_front();
                    return Ok(self.accept(queue, item).with_dropped_oldest());
                },
                Overflow::DropNewest => return Ok(PushOutcome::DroppedNewest),
                Overflow::Error => {
                    return Err(RuntimeError::Internal(
                        "stream buffer overflow (policy=error)".to_string(),
                    ));
                },
            }
        }
    }

    /// Insert the item, notify a consumer, and return the base outcome.
    fn accept(&self, mut queue: MutexGuard<'_, VecDeque<T>>, item: T) -> PushOutcome {
        queue.push_back(item);
        self.inner.not_empty.notify_one();
        PushOutcome::Accepted
    }

    /// Wait until another item is popped and buffer space becomes available.
    ///
    /// Registers the `Notified` future BEFORE releasing the queue lock so we
    /// cannot race past a `notify_one` that fires between `drop(queue)` and
    /// `.notified().await`. `Notify::notify_one` only stores a permit when a
    /// waiter is already registered; enabling the future via `as_mut().enable()`
    /// performs that registration without yielding. See tokio::sync::Notify docs.
    async fn wait_for_space(&self, queue: MutexGuard<'_, VecDeque<T>>) {
        let notified = self.inner.not_full.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        drop(queue);
        notified.await;
    }

    /// Receive next buffered item, waiting until one is available.
    pub async fn pop(&self) -> T {
        loop {
            let mut queue = self.inner.queue.lock().await;
            if let Some(item) = queue.pop_front() {
                self.inner.not_full.notify_one();
                return item;
            }
            // Same enable-before-drop pattern as `push`: register the
            // waiter while still holding the queue lock so a concurrent
            // `notify_one` cannot slip past us.
            let notified = self.inner.not_empty.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            drop(queue);
            notified.await;
        }
    }

    /// Current queue size.
    pub async fn len(&self) -> usize {
        self.inner.queue.lock().await.len()
    }

    /// Whether the queue currently has no buffered items.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}
