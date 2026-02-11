//! Span-like nested resource contexts example
//!
//! Demonstrates how resources automatically merge from parent contexts,
//! similar to how `tracing` spans inherit attributes.

use nebula_log::observability::*;
use tracing::Level;

fn main() {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    println!("=== Span-Like Nested Resources Example ===\n");

    // Register resource-aware webhook hook
    struct WebhookHook;
    impl ResourceAwareHook for WebhookHook {
        fn on_event_with_context(
            &self,
            event: &dyn ObservabilityEvent,
            _ctx: Option<std::sync::Arc<NodeContext>>,
        ) {
            if let Some(merged) = get_current_logger_resource() {
                println!("\n[WEBHOOK HOOK] Event: {}", event.name());
                println!("  Merged config:");
                if let Some(sentry) = merged.sentry_dsn() {
                    println!("    - Sentry: {}", sentry);
                }
                if let Some(webhook) = merged.webhook_url() {
                    println!("    - Webhook: {}", webhook);
                }
                println!("    - Log level: {:?}", merged.log_level);
                println!("    - Tags ({}): {:?}", merged.tags.len(), merged.tags);
            }
        }
    }

    register_hook(std::sync::Arc::new(ResourceAwareAdapter::new(WebhookHook)));

    println!("1. No context - no resources:");
    emit_event(&TestEvent("top_level".to_string()));

    println!("\n2. Execution context (like opening a tracing span):");
    ExecutionContext::new("exec-001", "workflow-order", "tenant-123")
        .with_resource(
            LoggerResource::new()
                .with_sentry_dsn("https://execution@sentry.io/project")
                .with_tag("execution_id", "exec-001")
                .with_tag("workflow_id", "workflow-order"),
        )
        .scope_sync(|| {
            emit_event(&TestEvent("execution_started".to_string()));

            println!("\n3. Nested node context (child span - inherits execution resources):");
            NodeContext::new("node-payment", "validate-card")
                .with_resource(
                    LoggerResource::new()
                        .with_webhook("https://hooks.slack.com/payment-team")
                        .with_tag("node_id", "node-payment")
                        .with_tag("action_id", "validate-card"),
                )
                .scope_sync(|| {
                    emit_event(&TestEvent("node_started".to_string()));

                    println!("\n4. Deeper nesting - another action (inherits both parent spans):");
                    NodeContext::new("node-payment", "charge-card")
                        .with_resource(
                            LoggerResource::new()
                                .with_log_level(LogLevel::Debug)
                                .with_tag("retry_count", "0"),
                        )
                        .scope_sync(|| {
                            emit_event(&TestEvent("charging".to_string()));
                        });

                    println!("\n5. Back to node level (action span closed):");
                    emit_event(&TestEvent("node_processing".to_string()));
                });

            println!("\n6. Back to execution level (node span closed):");
            emit_event(&TestEvent("execution_continuing".to_string()));
        });

    println!("\n7. Back to top level (all spans closed):");
    emit_event(&TestEvent("workflow_complete".to_string()));

    println!("\n\n=== Key Takeaways ===");
    println!("Contexts nest like `tracing` spans");
    println!("Child contexts inherit parent resources automatically");
    println!("Tags accumulate from all levels");
    println!("URLs/DSNs override (child replaces parent)");
    println!("When scope ends, context closes (like span.exit())");

    shutdown_hooks();
}

struct TestEvent(String);

impl ObservabilityEvent for TestEvent {
    fn name(&self) -> &str {
        &self.0
    }
}
