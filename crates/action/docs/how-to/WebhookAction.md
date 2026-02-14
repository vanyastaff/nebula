---
title: How to: WebhookAction
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: WebhookAction

**Purpose.** Receive inbound HTTP; manage subscribe/verify and secure handlers.

## Trait Signature
```rust
#[async_trait]
pub trait WebhookAction: Action {
    type Config: DeserializeOwned + Send + Sync;
    type Request: DeserializeOwned + Send + Sync;
    type Response: Serialize + Send + Sync;
    async fn subscribe(&self, cfg: Self::Config, wctx: &WebhookContext) -> Result<(), ActionError>;
    async fn handle(&self, req: Self::Request, wctx: &WebhookContext) -> Result<ActionResult<Self::Response>, ActionError>;
    async fn unsubscribe(&self) -> Result<(), ActionError>;
}
```

## Implementation Steps

1. `subscribe` to set up provider-side hooks; return endpoint/secrets via context if needed.
2. Verify signatures in `handle`; never log raw secrets.
3. Use idempotency keys to deduplicate repeated webhook deliveries.
4. `unsubscribe` on teardown or reconfiguration.


## Minimal Example
```rust
pub struct StripeWebhook;
#[derive(Deserialize)] pub struct Cfg { pub secret: String }
#[derive(Deserialize)] pub struct Request { pub payload: serde_json::Value, pub signature: String }
#[derive(Serialize)] pub struct Resp { pub accepted: bool }

#[async_trait]
impl WebhookAction for StripeWebhook {
    type Config = Cfg;
    type Request = Request;
    type Response = Resp;
    async fn subscribe(&self, _cfg: Cfg, _wctx: &WebhookContext) -> Result<(), ActionError> { Ok(()) }
    async fn handle(&self, req: Request, _wctx: &WebhookContext) -> Result<ActionResult<Resp>, ActionError> {
        // verify req.signature...
        Ok(ActionResult::Success(Resp { accepted: true }))
    }
    async fn unsubscribe(&self) -> Result<(), ActionError> { Ok(()) }
}
```
