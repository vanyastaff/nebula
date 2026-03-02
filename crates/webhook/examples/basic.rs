//! Example: Basic webhook trigger
//!
//! This example demonstrates how to create a simple webhook trigger
//! that accepts POST requests and emits events.

use async_trait::async_trait;
use nebula_resource::{Context, ExecutionId, Scope, WorkflowId};
use nebula_webhook::prelude::*;
use std::sync::Arc;

/// A simple trigger that accepts any POST request
struct SimpleTrigger;

#[async_trait]
impl WebhookAction for SimpleTrigger {
    type Event = String;

    async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()> {
        println!("✓ Webhook registered at: {}", ctx.webhook_url());
        Ok(())
    }

    async fn on_webhook(
        &self,
        _ctx: &TriggerCtx,
        payload: WebhookPayload,
    ) -> Result<Option<Self::Event>> {
        println!("📥 Received webhook: {} {}", payload.method, payload.path);

        // Parse body as string
        if let Some(body) = payload.body_str() {
            println!("   Body: {}", body);
            Ok(Some(body.to_string()))
        } else {
            Ok(None)
        }
    }

    async fn on_unsubscribe(&self, ctx: &TriggerCtx) -> Result<()> {
        println!("✗ Webhook unregistered from: {}", ctx.webhook_path());
        Ok(())
    }

    async fn test(&self, _ctx: &TriggerCtx) -> Result<TestResult> {
        Ok(TestResult::success("Simple trigger is ready"))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Configure server
    let config = WebhookServerConfig {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        base_url: "http://localhost:8080".to_string(),
        path_prefix: "/webhooks".to_string(),
        ..Default::default()
    };

    // Start server
    let server = WebhookServer::new(config).await?;
    println!("🚀 Webhook server started at http://localhost:8080");

    // Create trigger context
    let base = Context::new(Scope::Global, WorkflowId::v4(), ExecutionId::v4());
    let state = Arc::new(TriggerState::new("simple-trigger"));
    let ctx = TriggerCtx::new(
        base,
        "simple-trigger",
        Environment::Production,
        state,
        "http://localhost:8080",
        "/webhooks",
    );

    // Create trigger
    let trigger = SimpleTrigger;

    // Subscribe
    trigger.on_subscribe(&ctx).await?;
    let mut handle = server.subscribe(&ctx, None).await?;

    println!("\n📍 Send webhooks to: {}\n", ctx.webhook_url());
    println!("Example:");
    println!("  curl -X POST {} -d 'Hello, Nebula!'", ctx.webhook_url());
    println!("\nPress Ctrl+C to stop...\n");

    // Receive webhooks
    tokio::select! {
        _ = async {
            while let Ok(payload) = handle.recv().await {
                // Process webhook using trigger
                if let Ok(Some(event)) = trigger.on_webhook(&ctx, payload).await {
                    println!("✨ Event emitted: {}", event);
                }
            }
        } => {},
        _ = tokio::signal::ctrl_c() => {
            println!("\n\n🛑 Shutting down...");
        }
    }

    // Cleanup
    trigger.on_unsubscribe(&ctx).await?;
    drop(handle);
    server.shutdown().await?;

    println!("👋 Goodbye!");
    Ok(())
}
