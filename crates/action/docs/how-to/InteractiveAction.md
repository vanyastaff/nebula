---
title: How to: InteractiveAction
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: InteractiveAction

**Purpose.** Pause execution and await user/system input; resume via token.

## Trait Signature
```rust
#[async_trait]
pub trait InteractiveAction: Action {
    type Prompt: Serialize + DeserializeOwned + Send + Sync;
    type Reply: Serialize + DeserializeOwned + Send + Sync;
    async fn prompt(&self, prompt: Self::Prompt, ctx: &ExecutionContext)
        -> Result<ActionResult<PendingToken>, ActionError>;
    async fn resume(&self, token: PendingToken, reply: Self::Reply, ctx: &ExecutionContext)
        -> Result<ActionResult<()>, ActionError>;
}
```

## Implementation Steps

1. Issue a `PendingToken` with metadata to route replies.
2. Resume with validated `Reply`; enforce TTL and permissions.
3. Idempotency: ignore duplicate replies for same token.


## Minimal Example
```rust
pub struct Approval;
#[derive(Serialize, Deserialize)] pub struct Prompt { pub message: String }
#[derive(Serialize, Deserialize)] pub struct Reply { pub approved: bool }

#[async_trait]
impl InteractiveAction for Approval {
    type Prompt = Prompt;
    type Reply = Reply;
    async fn prompt(&self, prompt: Prompt, _ctx: &ExecutionContext) -> Result<ActionResult<PendingToken>, ActionError> {
        // send notification somewhere...
        Ok(ActionResult::Wait(WaitMode::PendingToken("approval-123".into())))
    }
    async fn resume(&self, _token: PendingToken, reply: Reply, _ctx: &ExecutionContext) -> Result<ActionResult<()>, ActionError> {
        if reply.approved { Ok(ActionResult::Success(())) } else { Ok(ActionResult::Stop) }
    }
}
```
