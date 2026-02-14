---
title: How to: QueueAction
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: QueueAction

**Purpose.** Enqueue work and process reliably with ack/nack + visibility timeouts.

## Trait Signature
```rust
#[async_trait]
pub trait QueueAction: Action {
    type Task: Serialize + DeserializeOwned + Send + Sync;
    async fn enqueue(&self, task: Self::Task, ctx: &ExecutionContext) -> Result<ActionResult<QueueId>, ActionError>;
    async fn dequeue(&self, ctx: &ExecutionContext) -> Result<ActionResult<DequeuedTask<Self::Task>>, ActionError>;
    async fn ack(&self, id: QueueId, ctx: &ExecutionContext) -> Result<(), ActionError>;
    async fn nack(&self, id: QueueId, requeue: bool, ctx: &ExecutionContext) -> Result<(), ActionError>;
}
```

## Implementation Steps

1. Use visibility timeouts to avoid duplicate processing.
2. Keep tasks small; store large payloads in object storage and pass references.
3. Record metrics: queue depth, processing latency, retries.


## Minimal Example
```rust
pub struct LocalQueue;
#[derive(Serialize, Deserialize)] pub struct Task { url: String }

#[async_trait]
impl QueueAction for LocalQueue {
    type Task = Task;
    async fn enqueue(&self, task: Task, _ctx: &ExecutionContext) -> Result<ActionResult<QueueId>, ActionError> {
        Ok(ActionResult::Enqueue(QueueId("q-1".into())))
    }
    async fn dequeue(&self, _ctx: &ExecutionContext) -> Result<ActionResult<DequeuedTask<Task>>, ActionError> { todo!() }
    async fn ack(&self, _id: QueueId, _ctx: &ExecutionContext) -> Result<(), ActionError> { Ok(()) }
    async fn nack(&self, _id: QueueId, _requeue: bool, _ctx: &ExecutionContext) -> Result<(), ActionError> { Ok(()) }
}
```
