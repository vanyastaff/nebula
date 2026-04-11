//! Durable inbound queue abstraction for the webhook server.
//!
//! The webhook crate defines its own minimal queue trait so it can remain in
//! the **API layer** without taking a dependency on `nebula-runtime` (Exec
//! layer). An adapter that bridges `nebula_runtime::queue::TaskQueue` to this
//! trait can live in the embedding application or in a future glue crate.
//!
//! # Durability contract
//!
//! Events are enqueued **before** the HTTP 200 response is sent.  This means
//! an event is persisted (or at least accepted by the queue implementation)
//! prior to acknowledging the sender — giving at-least-once delivery
//! semantics even if the Nebula process crashes immediately after.

use async_trait::async_trait;
use serde_json::Value;

/// Minimal durable queue interface for inbound webhook events.
///
/// Implementations must be `Send + Sync` to be usable behind an `Arc`.
///
/// # At-least-once semantics
///
/// The server calls [`enqueue`](Self::enqueue) before sending HTTP 200.  If
/// the call fails the server returns HTTP 500 so the sender will retry.
#[async_trait]
pub trait InboundQueue: Send + Sync {
    /// Persist one event payload.
    ///
    /// Returns a task identifier on success or a human-readable error string
    /// on failure.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if the queue is full, closed, or encounters an
    /// internal error.  The server will respond with HTTP 500 when this
    /// happens.
    async fn enqueue(&self, event: Value) -> Result<String, String>;
}

/// In-memory `InboundQueue` implementation for testing and development.
///
/// Uses an unbounded `Vec` protected by a `Mutex`; not suitable for
/// production workloads.
pub struct MemoryInboundQueue {
    items: std::sync::Mutex<Vec<Value>>,
}

impl MemoryInboundQueue {
    /// Create a new empty in-memory queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Drain all enqueued items (for use in tests).
    ///
    /// Returns the items that were in the queue, clearing it in the process.
    pub fn drain(&self) -> Vec<Value> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Number of items currently in the queue.
    pub fn len(&self) -> usize {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the queue is currently empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for MemoryInboundQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MemoryInboundQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.items.lock().map(|g| g.len()).unwrap_or(0);
        f.debug_struct("MemoryInboundQueue")
            .field("len", &len)
            .finish()
    }
}

#[async_trait]
impl InboundQueue for MemoryInboundQueue {
    async fn enqueue(&self, event: Value) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.items
            .lock()
            .map_err(|e| format!("mutex poisoned: {e}"))?
            .push(event);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn memory_queue_enqueue_and_drain() {
        let q = MemoryInboundQueue::new();
        assert!(q.is_empty());

        let id = q.enqueue(json!({"key": "value"})).await.unwrap();
        assert!(!id.is_empty());
        assert_eq!(q.len(), 1);

        let items = q.drain();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["key"], "value");
        assert!(q.is_empty());
    }

    #[tokio::test]
    async fn memory_queue_multiple_items() {
        let q = MemoryInboundQueue::new();
        q.enqueue(json!({"n": 1})).await.unwrap();
        q.enqueue(json!({"n": 2})).await.unwrap();
        q.enqueue(json!({"n": 3})).await.unwrap();
        assert_eq!(q.len(), 3);
        let items = q.drain();
        assert_eq!(items.len(), 3);
    }
}
