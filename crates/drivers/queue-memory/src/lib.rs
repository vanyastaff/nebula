#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Queue Memory Driver
//!
//! In-memory bounded task queue implementing the [`TaskQueue`] port.
//!
//! Uses `tokio::sync::mpsc` for the main queue channel and a `DashMap`
//! to track in-flight tasks for ack/nack semantics.
//!
//! Suitable for desktop and single-process deployments where durability
//! is not required.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_queue_memory::MemoryQueue;
//! use nebula_ports::TaskQueue;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let queue = MemoryQueue::new(1024);
//! let task_id = queue.enqueue(serde_json::json!({"action": "send_email"})).await?;
//! let item = queue.dequeue(Duration::from_secs(1)).await?;
//! if let Some((id, payload)) = item {
//!     // process task...
//!     queue.ack(&id).await?;
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nebula_ports::TaskQueue;
use nebula_ports::error::PortsError;
use tokio::sync::{Mutex, mpsc};

/// An item in the queue: task ID + payload.
#[derive(Debug, Clone)]
struct QueueItem {
    id: String,
    payload: serde_json::Value,
}

/// In-memory bounded task queue.
///
/// Tasks flow through three states:
/// 1. **Queued** -- sitting in the mpsc channel
/// 2. **In-flight** -- dequeued, awaiting ack/nack
/// 3. **Done** -- acked (removed) or nacked (requeued)
pub struct MemoryQueue {
    sender: mpsc::Sender<QueueItem>,
    receiver: Arc<Mutex<mpsc::Receiver<QueueItem>>>,
    in_flight: Arc<Mutex<HashMap<String, QueueItem>>>,
    queued_count: AtomicUsize,
}

impl MemoryQueue {
    /// Create a new memory queue with the given capacity.
    ///
    /// The capacity determines the maximum number of tasks that can be
    /// buffered in the queue. Enqueue will fail with `PortsError::Internal`
    /// when the queue is full.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::channel(capacity);
        Self {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            queued_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TaskQueue for MemoryQueue {
    async fn enqueue(&self, payload: serde_json::Value) -> Result<String, PortsError> {
        let id = uuid::Uuid::new_v4().to_string();
        let item = QueueItem {
            id: id.clone(),
            payload,
        };
        self.sender
            .try_send(item)
            .map_err(|e| PortsError::Internal(format!("queue full or closed: {e}")))?;
        self.queued_count.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    async fn dequeue(
        &self,
        timeout: Duration,
    ) -> Result<Option<(String, serde_json::Value)>, PortsError> {
        let mut rx = self.receiver.lock().await;
        let result = tokio::time::timeout(timeout, rx.recv()).await;
        match result {
            Ok(Some(item)) => {
                self.queued_count.fetch_sub(1, Ordering::Relaxed);
                let id = item.id.clone();
                let payload = item.payload.clone();
                self.in_flight.lock().await.insert(id.clone(), item);
                Ok(Some((id, payload)))
            }
            Ok(None) => {
                // Channel closed.
                Ok(None)
            }
            Err(_) => {
                // Timeout.
                Ok(None)
            }
        }
    }

    async fn ack(&self, task_id: &str) -> Result<(), PortsError> {
        let removed = self.in_flight.lock().await.remove(task_id);
        if removed.is_none() {
            return Err(PortsError::not_found("Task", task_id));
        }
        Ok(())
    }

    async fn nack(&self, task_id: &str) -> Result<(), PortsError> {
        let item = self.in_flight.lock().await.remove(task_id);
        match item {
            Some(item) => {
                self.sender
                    .try_send(item)
                    .map_err(|e| PortsError::Internal(format!("requeue failed: {e}")))?;
                self.queued_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            None => Err(PortsError::not_found("Task", task_id)),
        }
    }

    async fn len(&self) -> Result<usize, PortsError> {
        Ok(self.queued_count.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_and_dequeue() {
        let q = MemoryQueue::new(16);
        let payload = serde_json::json!({"key": "value"});
        let task_id = q.enqueue(payload.clone()).await.unwrap();
        assert!(!task_id.is_empty());

        let item = q.dequeue(Duration::from_secs(1)).await.unwrap();
        let (id, p) = item.expect("should dequeue a task");
        assert_eq!(id, task_id);
        assert_eq!(p, payload);
    }

    #[tokio::test]
    async fn dequeue_returns_none_on_timeout() {
        let q = MemoryQueue::new(16);
        let item = q.dequeue(Duration::from_millis(50)).await.unwrap();
        assert!(item.is_none());
    }

    #[tokio::test]
    async fn ack_removes_from_in_flight() {
        let q = MemoryQueue::new(16);
        let task_id = q.enqueue(serde_json::json!("test")).await.unwrap();
        let (id, _) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        assert_eq!(id, task_id);

        q.ack(&id).await.unwrap();

        // Double ack should fail.
        let result = q.ack(&id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn nack_requeues_task() {
        let q = MemoryQueue::new(16);
        let task_id = q.enqueue(serde_json::json!({"retry": true})).await.unwrap();

        let (id, _) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        assert_eq!(id, task_id);

        q.nack(&id).await.unwrap();

        // Should be able to dequeue again.
        let (id2, payload) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        assert_eq!(id2, task_id);
        assert_eq!(payload, serde_json::json!({"retry": true}));
    }

    #[tokio::test]
    async fn len_tracks_queued_count() {
        let q = MemoryQueue::new(16);
        assert_eq!(q.len().await.unwrap(), 0);
        assert!(q.is_empty().await.unwrap());

        q.enqueue(serde_json::json!(1)).await.unwrap();
        q.enqueue(serde_json::json!(2)).await.unwrap();
        assert_eq!(q.len().await.unwrap(), 2);

        let (id, _) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        assert_eq!(q.len().await.unwrap(), 1);

        q.ack(&id).await.unwrap();
        assert_eq!(q.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn enqueue_fails_when_full() {
        let q = MemoryQueue::new(1);
        q.enqueue(serde_json::json!("first")).await.unwrap();
        let result = q.enqueue(serde_json::json!("second")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn nack_unknown_task_returns_not_found() {
        let q = MemoryQueue::new(16);
        let result = q.nack("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fifo_ordering() {
        let q = MemoryQueue::new(16);
        q.enqueue(serde_json::json!(1)).await.unwrap();
        q.enqueue(serde_json::json!(2)).await.unwrap();
        q.enqueue(serde_json::json!(3)).await.unwrap();

        let (id1, p1) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        let (id2, p2) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();
        let (id3, p3) = q.dequeue(Duration::from_secs(1)).await.unwrap().unwrap();

        assert_eq!(p1, serde_json::json!(1));
        assert_eq!(p2, serde_json::json!(2));
        assert_eq!(p3, serde_json::json!(3));

        q.ack(&id1).await.unwrap();
        q.ack(&id2).await.unwrap();
        q.ack(&id3).await.unwrap();
    }
}
