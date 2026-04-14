//! Task queue interface and in-memory implementation.
//!
//! Used to distribute work to workers; at-least-once delivery with ack/nack.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

/// Errors returned by queue operations.
#[derive(Debug, Error)]
pub enum QueueError {
    /// Task not found (e.g. ack/nack unknown id).
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity type (for example, `task`).
        entity: String,
        /// Missing entity identifier.
        id: String,
    },

    /// Internal queue failure (full, closed, etc.).
    #[error("internal error: {0}")]
    Internal(String),
}

impl QueueError {
    /// Convenience constructor for [`QueueError::NotFound`].
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }
}

/// Work queue interface for distributing tasks to workers.
///
/// At-least-once semantics: enqueue → dequeue → ack (or nack to requeue).
#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Enqueue a task. Returns a task ID.
    async fn enqueue(&self, payload: serde_json::Value) -> Result<String, QueueError>;

    /// Dequeue the next available task. Returns `(task_id, payload)` or `None` on timeout.
    async fn dequeue(
        &self,
        timeout: Duration,
    ) -> Result<Option<(String, serde_json::Value)>, QueueError>;

    /// Acknowledge successful processing.
    async fn ack(&self, task_id: &str) -> Result<(), QueueError>;

    /// Negative-acknowledge — requeue for retry.
    async fn nack(&self, task_id: &str) -> Result<(), QueueError>;

    /// Number of tasks currently in the queue.
    async fn len(&self) -> Result<usize, QueueError>;

    /// Whether the queue is empty.
    async fn is_empty(&self) -> Result<bool, QueueError> {
        Ok(self.len().await? == 0)
    }
}

#[derive(Debug, Clone)]
struct QueueItem {
    id: String,
    payload: serde_json::Value,
}

/// In-memory bounded task queue.
///
/// Tasks: Queued → In-flight (dequeued) → Done (acked) or requeued (nacked).
pub struct MemoryQueue {
    sender: mpsc::Sender<QueueItem>,
    receiver: Arc<Mutex<mpsc::Receiver<QueueItem>>>,
    in_flight: Arc<Mutex<HashMap<String, QueueItem>>>,
    queued_count: AtomicUsize,
}

impl MemoryQueue {
    /// Create a new memory queue with the given capacity.
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
    async fn enqueue(&self, payload: serde_json::Value) -> Result<String, QueueError> {
        let id = uuid::Uuid::new_v4().to_string();
        let item = QueueItem {
            id: id.clone(),
            payload,
        };
        self.sender
            .try_send(item)
            .map_err(|e| QueueError::Internal(format!("queue full or closed: {e}")))?;
        self.queued_count.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    async fn dequeue(
        &self,
        timeout: Duration,
    ) -> Result<Option<(String, serde_json::Value)>, QueueError> {
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
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    async fn ack(&self, task_id: &str) -> Result<(), QueueError> {
        let removed = self.in_flight.lock().await.remove(task_id);
        if removed.is_none() {
            return Err(QueueError::not_found("Task", task_id));
        }
        Ok(())
    }

    async fn nack(&self, task_id: &str) -> Result<(), QueueError> {
        // Keep the item in-flight until requeue succeeds to preserve
        // at-least-once guarantees when the queue is saturated.
        let item = {
            let in_flight = self.in_flight.lock().await;
            in_flight.get(task_id).cloned()
        };
        let Some(item) = item else {
            return Err(QueueError::not_found("Task", task_id));
        };

        self.sender
            .send(item)
            .await
            .map_err(|e| QueueError::Internal(format!("requeue failed: {e}")))?;
        self.queued_count.fetch_add(1, Ordering::Relaxed);
        let _ = self.in_flight.lock().await.remove(task_id);
        Ok(())
    }

    async fn len(&self) -> Result<usize, QueueError> {
        Ok(self.queued_count.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nack_waits_for_capacity_and_preserves_task() {
        let queue = Arc::new(MemoryQueue::new(1));

        let first_id = queue
            .enqueue(serde_json::json!({"task":"first"}))
            .await
            .unwrap();
        let (dequeued_id, _) = queue
            .dequeue(Duration::from_millis(50))
            .await
            .unwrap()
            .expect("expected dequeued task");
        assert_eq!(dequeued_id, first_id);

        // Fill the queue so nack must wait for capacity.
        queue
            .enqueue(serde_json::json!({"task":"filler"}))
            .await
            .unwrap();

        let queue_for_nack = Arc::clone(&queue);
        let id_for_nack = dequeued_id.clone();
        let nack_task = tokio::spawn(async move { queue_for_nack.nack(&id_for_nack).await });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !nack_task.is_finished(),
            "nack should block while queue is full"
        );

        // Free one slot, then nack should complete and requeue original task.
        let (_filler_id, _filler_payload) = queue
            .dequeue(Duration::from_millis(50))
            .await
            .unwrap()
            .expect("expected filler dequeue");
        nack_task.await.unwrap().unwrap();

        let (requeued_id, _) = queue
            .dequeue(Duration::from_millis(50))
            .await
            .unwrap()
            .expect("expected requeued task");
        assert_eq!(requeued_id, dequeued_id);
    }
}
