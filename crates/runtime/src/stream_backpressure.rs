//! Bounded stream buffer with explicit backpressure policies.
//!
//! This is a runtime-level primitive for stream-oriented action outputs where
//! producer and consumer rates may diverge.

use std::{collections::VecDeque, sync::Arc};

use nebula_action::Overflow;
use tokio::sync::{Mutex, Notify};

use crate::RuntimeError;

/// Result of pushing an item into a bounded stream buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// Item was accepted without eviction.
    Accepted,
    /// Item was accepted after evicting one buffered item.
    AcceptedAfterDropOldest,
    /// Item was dropped due to `Overflow::DropNewest`.
    DroppedNewest,
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
        let mut item = Some(item);

        loop {
            let mut queue = self.inner.queue.lock().await;

            if queue.len() < self.inner.capacity {
                queue.push_back(item.take().expect("item available"));
                self.inner.not_empty.notify_one();
                return Ok(PushOutcome::Accepted);
            }

            match self.inner.overflow {
                Overflow::Block => {
                    drop(queue);
                    self.inner.not_full.notified().await;
                }
                Overflow::DropOldest => {
                    let _ = queue.pop_front();
                    queue.push_back(item.take().expect("item available"));
                    self.inner.not_empty.notify_one();
                    return Ok(PushOutcome::AcceptedAfterDropOldest);
                }
                Overflow::DropNewest => {
                    return Ok(PushOutcome::DroppedNewest);
                }
                Overflow::Error => {
                    return Err(RuntimeError::Internal(
                        "stream buffer overflow (policy=error)".to_string(),
                    ));
                }
            }
        }
    }

    /// Receive next buffered item, waiting until one is available.
    pub async fn pop(&self) -> T {
        loop {
            let mut queue = self.inner.queue.lock().await;
            if let Some(item) = queue.pop_front() {
                self.inner.not_full.notify_one();
                return item;
            }
            drop(queue);
            self.inner.not_empty.notified().await;
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
