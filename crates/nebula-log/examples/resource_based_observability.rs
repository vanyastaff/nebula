//! Comprehensive example demonstrating resource-based observability
//!
//! This example shows:
//! - Multi-context system (Global, Execution, Node)
//! - Resource-based security (LoggerResource scoped per-node)
//! - Lock-free event emission
//! - Panic safety
//! - Event filtering
//! - Resource-aware hooks

use nebula_log::observability::*;
use std::sync::Arc;
use tracing::Level;

/// Custom event for workflow operations
#[derive(Debug)]
struct WorkflowEvent {
    name: String,
    workflow_id: String,
}

impl ObservabilityEvent for WorkflowEvent {
    fn name(&self) -> &str {
        &self.name
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "workflow_id": self.workflow_id,
        }))
    }
}

/// Custom hook that sends notifications to webhooks based on LoggerResource
struct WebhookNotificationHook;

impl ResourceAwareHook for WebhookNotificationHook {
    fn on_event_with_context(&self, event: &dyn ObservabilityEvent, ctx: Option<Arc<NodeContext>>) {
        if let Some(ctx) = ctx {
            // Access LoggerResource from node context (if attached)
            if let Some(logger) = ctx.get_resource::<LoggerResource>() {
                if let Some(webhook) = logger.webhook_url() {
                    // In a real implementation, this would send an HTTP POST
                    println!(
                        "[WEBHOOK] Node {} - Event: {} -> {}",
                        ctx.node_id,
                        event.name(),
                        webhook
                    );
                }
            }
        }
    }
}

/// Custom hook with event filtering - only processes workflow.* events
struct FilteredWorkflowHook {
    filter: EventFilter,
}

impl FilteredWorkflowHook {
    fn new() -> Self {
        Self {
            filter: EventFilter::prefix("workflow."),
        }
    }
}

impl ObservabilityHook for FilteredWorkflowHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        if self.filter.matches(event) {
            println!("[WORKFLOW HOOK] Processing: {}", event.name());
        }
    }
}

fn main() {
    // Initialize tracing for the logging hook
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    println!("=== Resource-Based Observability Example ===\n");

    // Step 1: Set up global context (application-wide)
    println!("1. Setting up global context...");
    let global_ctx =
        GlobalContext::new("nebula-workflow", "0.1.0", "production").with_instance_id("worker-1");
    let _global_guard = global_ctx.set_current();
    println!(
        "   ✓ Global context set: {} v{}\n",
        "nebula-workflow", "0.1.0"
    );

    // Step 2: Register observability hooks
    println!("2. Registering hooks...");

    // Basic logging hook
    let logging_hook = LoggingHook::new(Level::INFO);
    register_hook(Arc::new(logging_hook));
    println!("   ✓ Registered LoggingHook");

    // Resource-aware webhook hook (wrapped in adapter)
    let webhook_hook = ResourceAwareAdapter::new(WebhookNotificationHook);
    register_hook(Arc::new(webhook_hook));
    println!("   ✓ Registered WebhookNotificationHook (resource-aware)");

    // Filtered workflow hook
    let filtered_hook = FilteredWorkflowHook::new();
    register_hook(Arc::new(filtered_hook));
    println!("   ✓ Registered FilteredWorkflowHook (workflow.* events only)\n");

    // Step 3: Start workflow execution context
    println!("3. Starting workflow execution...");
    let exec_ctx = ExecutionContext::new("exec-001", "wf-order-processing", "tenant-123")
        .with_trace_id("trace-abc-def");
    let _exec_guard = exec_ctx.enter();
    println!("   ✓ Execution context entered: exec-001\n");

    // Emit workflow event (will be processed by all hooks)
    println!("4. Emitting workflow.started event...");
    emit_event(&WorkflowEvent {
        name: "workflow.started".to_string(),
        workflow_id: "wf-order-processing".to_string(),
    });
    println!();

    // Step 4: Execute first node WITHOUT LoggerResource (no webhook notification)
    println!("5. Executing node WITHOUT LoggerResource...");
    {
        let node_ctx = NodeContext::new("node-validation", "validation.schema");
        let _node_guard = node_ctx.enter();

        emit_event(&WorkflowEvent {
            name: "node.validation.started".to_string(),
            workflow_id: "wf-order-processing".to_string(),
        });
        println!("   (Note: No webhook notification - LoggerResource not attached)\n");
    }

    // Step 5: Execute second node WITH LoggerResource (webhook notification sent)
    println!("6. Executing node WITH LoggerResource...");
    {
        // Create LoggerResource with webhook configured
        let logger = LoggerResource::new()
            .with_log_level(LogLevel::Debug)
            .with_webhook("https://hooks.slack.com/services/TENANT123/CHANNEL")
            .with_sentry_dsn("https://key@sentry.io/project-tenant123")
            .with_tag("environment", "production")
            .with_tag("tenant_id", "tenant-123");

        // Attach LoggerResource to node context (SECURE: scoped to this node only)
        let node_ctx = NodeContext::new("node-payment", "payment.process").with_resource(logger);

        let _node_guard = node_ctx.enter();

        emit_event(&WorkflowEvent {
            name: "node.payment.started".to_string(),
            workflow_id: "wf-order-processing".to_string(),
        });
        println!("   (Note: Webhook notification sent - LoggerResource attached)\n");
    }

    // Step 6: Execute third node with DIFFERENT LoggerResource (isolated)
    println!("7. Executing another node with DIFFERENT LoggerResource...");
    {
        let logger = LoggerResource::new()
            .with_webhook("https://different-webhook.com/tenant456")
            .with_tag("tenant_id", "tenant-456");

        let node_ctx = NodeContext::new("node-inventory", "inventory.check").with_resource(logger);

        let _node_guard = node_ctx.enter();

        emit_event(&WorkflowEvent {
            name: "node.inventory.started".to_string(),
            workflow_id: "wf-order-processing".to_string(),
        });
        println!("   (Note: Different webhook - resources are ISOLATED per node)\n");
    }

    // Step 7: Emit workflow completion
    println!("8. Workflow completed:");
    emit_event(&WorkflowEvent {
        name: "workflow.completed".to_string(),
        workflow_id: "wf-order-processing".to_string(),
    });
    println!();

    // Step 8: Demonstrate panic safety
    println!("9. Testing panic safety...");
    struct PanickingHook;
    impl ObservabilityHook for PanickingHook {
        fn on_event(&self, event: &dyn ObservabilityEvent) {
            if event.name() == "test.panic" {
                panic!("Intentional panic");
            }
        }
    }

    register_hook(Arc::new(PanickingHook));
    emit_event(&WorkflowEvent {
        name: "test.panic".to_string(),
        workflow_id: "test".to_string(),
    });
    println!("   ✓ System survived panicking hook (caught and logged)\n");

    // Step 9: Demonstrate event filtering
    println!("10. Testing event filtering:");
    emit_event(&WorkflowEvent {
        name: "node.test".to_string(), // Won't match workflow.* filter
        workflow_id: "test".to_string(),
    });
    println!("    (Note: FilteredWorkflowHook did NOT process node.test event)\n");

    emit_event(&WorkflowEvent {
        name: "workflow.test".to_string(), // Will match workflow.* filter
        workflow_id: "test".to_string(),
    });
    println!("    (Note: FilteredWorkflowHook processed workflow.test event)\n");

    // Cleanup
    println!("11. Shutting down hooks...");
    shutdown_hooks();
    println!("    ✓ All hooks shut down\n");

    println!("=== Example Complete ===");
    println!("\nKey Takeaways:");
    println!("1. ✅ LoggerResource is scoped PER-NODE (secure, isolated)");
    println!("2. ✅ Nodes without LoggerResource work fine (optional configuration)");
    println!("3. ✅ Different nodes can have different LoggerResource configs");
    println!("4. ✅ System survives panicking hooks (panic safety)");
    println!("5. ✅ Event filtering reduces overhead for specific hooks");
    println!("6. ✅ Lock-free emission allows concurrent event processing");
}
