# nebula-webhook

High-performance webhook server infrastructure for the Nebula workflow engine.

## Overview

`nebula-webhook` provides a unified HTTP webhook server that enables external services (Telegram, GitHub, Stripe, etc.) to trigger workflows through unique, isolated webhook endpoints.

## Features

- **🎯 Single Server Architecture**: One HTTP server per runtime, handling all webhooks
- **🔒 UUID Isolation**: Each trigger gets a unique, unpredictable UUID path
- **🌍 Environment Separation**: Test and Production traffic completely isolated
- **♻️ RAII Lifecycle**: Automatic cleanup when triggers are dropped
- **🎨 Framework Abstraction**: Developers only implement business logic
- **⚡ High Performance**: Built on Axum and Tokio for maximum throughput
- **📊 Observable**: Built-in tracing and metrics support

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Nebula Runtime                           │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    WebhookServer                        │   │
│  │                  (singleton, Arc<T>)                    │   │
│  │                                                         │   │
│  │   TcpListener :8080                                     │   │
│  │       │                                                 │   │
│  │   axum Router  /*path  ──►  handler()                   │   │
│  │                                  │                      │   │
│  │   RouteMap: HashMap<             │                      │   │
│  │     String,                      ▼                      │   │
│  │     broadcast::Sender<WebhookPayload>                   │   │
│  │   >                                                     │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Usage

### Basic Example

```rust
use nebula_webhook::prelude::*;
use async_trait::async_trait;

struct TelegramTrigger;

#[async_trait]
impl WebhookAction for TelegramTrigger {
    type Event = String;

    async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()> {
        // Register webhook with Telegram
        let url = ctx.webhook_url();
        telegram::set_webhook(url).await?;
        Ok(())
    }

    async fn on_webhook(
        &self,
        ctx: &TriggerCtx,
        payload: WebhookPayload,
    ) -> Result<Option<Self::Event>> {
        // Verify signature
        if !verify_telegram_signature(&payload) {
            return Ok(None); // Filter out invalid requests
        }

        // Parse event
        let event = String::from_utf8(payload.body.to_vec()).unwrap();
        Ok(Some(event))
    }

    async fn on_unsubscribe(&self, ctx: &TriggerCtx) -> Result<()> {
        // Clean up webhook
        telegram::delete_webhook().await?;
        Ok(())
    }

    async fn test(&self, ctx: &TriggerCtx) -> Result<TestResult> {
        // Test connectivity
        let info = telegram::get_me().await?;
        Ok(TestResult::success(format!("Connected: {}", info.username)))
    }
}
```

### Server Setup

```rust
use nebula_webhook::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure server
    let config = WebhookServerConfig {
        bind_addr: "0.0.0.0:8080".parse().unwrap(),
        base_url: "https://nebula.example.com".to_string(),
        path_prefix: "/webhooks".to_string(),
        enable_compression: true,
        enable_cors: true,
        body_limit: 10 * 1024 * 1024, // 10 MB
    };

    // Start server
    let server = WebhookServer::new(config).await?;

    // Server is now running and accepting webhooks
    println!("Webhook server listening on {}", server.config().bind_addr);

    Ok(())
}
```

### Subscribing to Webhooks

```rust
use nebula_webhook::prelude::*;
use nebula_resource::Context;
use nebula_resource::Scope;
use std::sync::Arc;

async fn subscribe_example(server: Arc<WebhookServer>) -> Result<()> {
    // Create context
    let base = Context::new(Scope::Global, "workflow-1", "execution-1");
    let state = Arc::new(TriggerState::new("my-trigger"));
    
    let ctx = TriggerCtx::new(
        base,
        "my-trigger",
        Environment::Production,
        state,
        "https://nebula.example.com",
        "/webhooks",
    );

    // Subscribe to webhooks
    let mut handle = server.subscribe(&ctx, None).await?;

    // Receive webhooks
    while let Ok(payload) = handle.recv().await {
        println!("Received webhook: {:?}", payload);
    }

    Ok(())
}
```

## Path Structure

Webhook paths follow this format:

```
/{path_prefix}/{environment}/{uuid}
```

Examples:
- `/webhooks/test/550e8400-e29b-41d4-a716-446655440000` (Test)
- `/webhooks/prod/7c9e6679-7425-40de-944b-e07fc1f90ae7` (Production)

Each trigger has **two UUIDs** - one for test and one for production. These are generated once and persisted, ensuring stable webhook URLs across restarts.

## Environment Isolation

| | Test | Production |
|---|---|---|
| UUID | `test_uuid` | `prod_uuid` |
| Path | `/webhooks/test/{uuid}` | `/webhooks/prod/{uuid}` |
| Credentials | Test keys | Production keys |
| Traffic | Isolated | Isolated |
| `test()` method | ✅ Always here | ❌ Never |

## RAII Lifecycle

Webhook triggers use RAII for automatic cleanup:

```rust
{
    // Subscribe creates a TriggerHandle
    let handle = server.subscribe(&ctx, None).await?;
    
    // Use the handle...
    
} // <- Handle dropped: webhook unregistered, cleanup called automatically
```

## Features

- `metrics` - Enable Prometheus metrics collection

## Testing

```bash
cargo test --package nebula-webhook
cargo test --package nebula-webhook --all-features
```

## Performance

- **Zero-copy** payload handling with `bytes::Bytes`
- **Broadcast channels** for efficient multi-consumer distribution
- **Lock-free routing** with `DashMap`
- **Async I/O** throughout with Tokio

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE))
- MIT license ([LICENSE-MIT](../../LICENSE-MIT))

at your option.
