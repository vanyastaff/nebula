---
title: How to: StreamingAction
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: StreamingAction

**Purpose.** Stream items/chunks with backpressure & checkpoints.

## Trait Signature
```rust
#[async_trait]
pub trait StreamingAction: Action {
    type Config: DeserializeOwned + Send + Sync;
    type Chunk: Serialize + Send + Sync;
    async fn open_stream(&self, cfg: Self::Config, ctx: &ExecutionContext) -> Result<StreamHandle, ActionError>;
    async fn next_chunk(&self, handle: &mut StreamHandle, ctx: &ExecutionContext)
        -> Result<StreamStep<Self::Chunk>, ActionError>;
    async fn close(&self, handle: StreamHandle) -> Result<(), ActionError>;
}
```

## Implementation Steps

1. Use `open_stream` to allocate sources, `next_chunk` to pull respecting backpressure.
2. Emit `StreamStep::Item(chunk)` or `::Done`; include checkpoints for resume.
3. Close gracefully to release resources.


## Minimal Example
```rust
pub struct FileStreamer;

#[async_trait]
impl StreamingAction for FileStreamer {
    type Config = String; // path
    type Chunk = bytes::Bytes;
    async fn open_stream(&self, _cfg: String, _ctx: &ExecutionContext) -> Result<StreamHandle, ActionError> { todo!() }
    async fn next_chunk(&self, _h: &mut StreamHandle, _ctx: &ExecutionContext) -> Result<StreamStep<bytes::Bytes>, ActionError> { todo!() }
    async fn close(&self, _h: StreamHandle) -> Result<(), ActionError> { Ok(()) }
}
```
