---
title: "How to: PollingAction"
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: PollingAction

**Purpose.** Periodic fetch with cursor + lease; avoids duplicates and supports backoff.

## Trait Signature
```rust
#[async_trait]
pub trait PollingAction: Action {
    type Config: DeserializeOwned + Send + Sync;
    type Cursor: Serialize + DeserializeOwned + Default + Send + Sync;
    type Item: Serialize + Send + Sync;
    async fn poll(&self, cfg: Self::Config, cursor: &mut Self::Cursor, lease: &Lease, ctx: &ExecutionContext)
        -> Result<PollResult<Self::Item, Self::Cursor>, ActionError>;
}
```

## Implementation Steps

1. Model `Cursor` with last_id/updated_at/page_token.
2. Use `Lease` to bound runtime per tick; return `backoff` when done.
3. Emit items in small batches; engine fan-outs downstream.
4. Persist updated `Cursor` in the returned result.


## Minimal Example
```rust
#[derive(Default, Serialize, Deserialize)] pub struct Cursor { last_id: Option<String> }
#[derive(Deserialize)] pub struct Cfg { base_url: String, page_size: u32 }
#[derive(Serialize)] pub struct Item { id: String, payload: serde_json::Value }

pub struct ApiPoller;

#[async_trait]
impl PollingAction for ApiPoller {
    type Config = Cfg;
    type Cursor = Cursor;
    type Item = Item;
    async fn poll(&self, cfg: Cfg, cursor: &mut Cursor, lease: &Lease, _ctx: &ExecutionContext)
        -> Result<PollResult<Item, Cursor>, ActionError> {
        // fetch page after cursor.last_id, respecting lease.remaining()
        Ok(PollResult::items(vec![], cursor.clone()).with_backoff_ms(5000))
    }
}
```
