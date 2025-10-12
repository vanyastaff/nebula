//! Span-like nested resource contexts example
//!
//! Demonstrates how resources automatically merge from parent contexts,
//! similar to how `tracing` spans inherit attributes.

use nebula_log::observability::*;
use tracing::Level;

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    println!("=== Span-Like Nested Resources Example ===\n");

    // Register resource-aware webhook hook
    struct WebhookHook;
    impl ResourceAwareHook for WebhookHook {
        fn on_event_with_context(
            &self,
            event: &dyn ObservabilityEvent,
            ctx: Option<std::sync::Arc<NodeContext>>,
        ) {
            // Get merged resource from ALL parent spans
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

    register_hook(std::sync::Arc::new(ResourceAwareAdapter::new(
        WebhookHook,
    )));

    println!("1. No context - no resources:");
    emit_event(&TestEvent("top_level".to_string()));

    println!("\n2. Execution context (like opening a tracing span):");
    {
        let exec = ExecutionContext::new("exec-001", "workflow-order", "tenant-123")
            .with_resource(
                "LoggerResource",
                LoggerResource::new()
                    .with_sentry_dsn("https://execution@sentry.io/project")
                    .with_tag("execution_id", "exec-001")
                    .with_tag("workflow_id", "workflow-order"),
            );

        let _exec_guard = exec.enter(); // ← Opens execution "span"

        emit_event(&TestEvent("execution_started".to_string()));

        println!("\n3. Nested node context (child span - inherits execution resources):");
        {
            let node = NodeContext::new("node-payment", "validate-card").with_resource(
                "LoggerResource",
                LoggerResource::new()
                    .with_webhook("https://hooks.slack.com/payment-team")
                    .with_tag("node_id", "node-payment")
                    .with_tag("action_id", "validate-card"),
            );

            let _node_guard = node.enter(); // ← Opens node "span" (inherits exec resources!)

            emit_event(&TestEvent("node_started".to_string()));
            // This event has:
            // - Sentry from Execution
            // - Webhook from Node
            // - Tags from BOTH (accumulated!)

            println!("\n4. Deeper nesting - another action (inherits both parent spans):");
            {
                let action = NodeContext::new("node-payment", "charge-card").with_resource(
                    "LoggerResource",
                    LoggerResource::new()
                        .with_log_level(LogLevel::Debug) // Override log level
                        .with_tag("retry_count", "0"),
                );

                let _action_guard = action.enter(); // ← Even deeper span!

                emit_event(&TestEvent("charging".to_string()));
                // This event has:
                // - Sentry from Execution
                // - Webhook from Node (parent)
                // - Log level from THIS action (override)
                // - Tags from ALL three levels!
            }

            println!("\n5. Back to node level (action span closed):");
            emit_event(&TestEvent("node_processing".to_string()));
            // Back to node + execution resources (no action override)
        }

        println!("\n6. Back to execution level (node span closed):");
        emit_event(&TestEvent("execution_continuing".to_string()));
        // Only execution resources (no node/action)
    }

    println!("\n7. Back to top level (all spans closed):");
    emit_event(&TestEvent("workflow_complete".to_string()));
    // No resources again

    println!("\n\n=== Key Takeaways ===");
    println!("✅ Contexts nest like `tracing` spans");
    println!("✅ Child contexts inherit parent resources automatically");
    println!("✅ Tags accumulate from all levels");
    println!("✅ URLs/DSNs override (child replaces parent)");
    println!("✅ When guard drops, context closes (like span.exit())");
    println!("✅ Multiple actions in same node? Just open/close node spans!");

    println!("\n\n=== Your Use Case: Node with Multiple Actions ===");
    println!("
// Open node span ONCE
let node = NodeContext::new(\"payment-node\", \"\")
    .with_resource(\"LoggerResource\", node_level_logger); // ← For ALL actions

let _node_guard = node.enter();

// Action 1: Inherits node resources
{{
    let action1 = NodeContext::new(\"payment-node\", \"validate\")
        .with_resource(\"LoggerResource\", LoggerResource::new()
            .with_tag(\"action\", \"validate\")
        );
    let _g = action1.enter();
    // Has: node's Sentry + node's webhook + action's tag
    execute_action_1();
}}

// Action 2: Different override
{{
    let action2 = NodeContext::new(\"payment-node\", \"charge\")
        .with_resource(\"LoggerResource\", LoggerResource::new()
            .with_webhook(\"https://different-webhook.com\") // Override!
            .with_tag(\"action\", \"charge\")
        );
    let _g = action2.enter();
    // Has: node's Sentry + THIS webhook (override) + both tags
    execute_action_2();
}}

// Back to node level
emit_event(&NodeComplete);
");

    shutdown_hooks();
}

struct TestEvent(String);

impl ObservabilityEvent for TestEvent {
    fn name(&self) -> &str {
        &self.0
    }
}
