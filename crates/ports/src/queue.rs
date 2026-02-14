//! Task queue port.
//!
//! Defines the interface for enqueuing and dequeuing work items.
//! Backend drivers (in-memory, Redis, SQS) implement this trait.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::PortsError;

/// Work queue interface for distributing tasks to workers.
///
/// Follows at-least-once delivery semantics:
/// - [`enqueue`](Self::enqueue) adds a task and returns its ID
/// - [`dequeue`](Self::dequeue) retrieves the next task (blocking up to `timeout`)
/// - [`ack`](Self::ack) confirms successful processing
/// - [`nack`](Self::nack) requeues a task for retry
#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Enqueue a task. Returns a task ID.
    async fn enqueue(&self, payload: serde_json::Value) -> Result<String, PortsError>;

    /// Dequeue the next available task. Returns `(task_id, payload)` or `None` on timeout.
    async fn dequeue(
        &self,
        timeout: Duration,
    ) -> Result<Option<(String, serde_json::Value)>, PortsError>;

    /// Acknowledge successful processing.
    async fn ack(&self, task_id: &str) -> Result<(), PortsError>;

    /// Negative-acknowledge -- requeue for retry.
    async fn nack(&self, task_id: &str) -> Result<(), PortsError>;

    /// Number of tasks currently in the queue.
    async fn len(&self) -> Result<usize, PortsError>;

    /// Whether the queue is empty. Default implementation calls [`len`](Self::len).
    async fn is_empty(&self) -> Result<bool, PortsError> {
        Ok(self.len().await? == 0)
    }
}
