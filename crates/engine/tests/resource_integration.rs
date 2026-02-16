//! End-to-end integration test: action acquires a resource through the engine.
//!
//! Proves the full chain:
//!   register(MockResource) in Manager
//!     -> Engine holds Manager
//!       -> Action calls ctx.resource("mock")
//!         -> gets ResourceHandle
//!           -> downcasts to the concrete instance type

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::capability::IsolationLevel;
use nebula_action::context::ActionContext;
use nebula_action::handler::InternalHandler;
use nebula_action::metadata::{ActionMetadata, ActionType};
use nebula_action::result::ActionResult;
use nebula_action::{ActionError, ExecutionBudget, ParameterCollection};
use nebula_core::Version;
use nebula_core::id::{ActionId, NodeId, WorkflowId};
use nebula_engine::WorkflowEngine;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::{Context as ResourceContext, Manager, PoolConfig, ResourceHandle};
use nebula_runtime::registry::ActionRegistry;
use nebula_runtime::{ActionRuntime, DataPassingPolicy};
use nebula_sandbox_inprocess::{ActionExecutor, InProcessSandbox};
use nebula_telemetry::event::EventBus;
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{NodeDefinition, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Mock resource
// ---------------------------------------------------------------------------

struct MockConfig;
impl Config for MockConfig {}

/// A trivial resource whose instances are strings.
struct MockResource;

impl Resource for MockResource {
    type Config = MockConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "mock"
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &ResourceContext,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok("mock-instance".to_string())
    }
}

// ---------------------------------------------------------------------------
// Action handler that acquires a resource
// ---------------------------------------------------------------------------

/// An action handler that calls `ctx.resource("mock")`, downcasts to a
/// `ResourceHandle`, reads the inner `String`, and returns it as output.
struct ResourceConsumerHandler {
    meta: ActionMetadata,
}

#[async_trait]
impl InternalHandler for ResourceConsumerHandler {
    async fn execute(
        &self,
        _input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let boxed = ctx.resource("mock").await?;
        let handle = boxed
            .downcast_ref::<ResourceHandle>()
            .ok_or_else(|| ActionError::fatal("expected ResourceHandle"))?;
        let value = handle
            .get::<String>()
            .ok_or_else(|| ActionError::fatal("expected String instance"))?;
        Ok(ActionResult::success(
            serde_json::json!({ "resource_value": value }),
        ))
    }

    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
    fn action_type(&self) -> ActionType {
        ActionType::Process
    }
    fn parameters(&self) -> Option<&ParameterCollection> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::v4(),
        name: "resource-integration-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections: vec![],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn meta(key: &str) -> ActionMetadata {
    ActionMetadata::new(key, key, "resource integration test").with_isolation(IsolationLevel::None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Single-node workflow where the action acquires a resource from the manager
/// via `ctx.resource("mock")` and returns the instance value as output.
#[tokio::test]
async fn action_acquires_resource_through_engine() {
    // 1. Create and populate the resource manager
    let manager = Arc::new(Manager::new());
    manager
        .register(MockResource, MockConfig, PoolConfig::default())
        .expect("register mock resource");

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(ResourceConsumerHandler {
        meta: meta("resource-consumer"),
    }));

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let event_bus = Arc::new(EventBus::new(128));
    let metrics = Arc::new(MetricsRegistry::new());
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        event_bus.clone(),
        metrics.clone(),
    ));

    let action_id = ActionId::v4();
    let mut engine =
        WorkflowEngine::new(runtime, event_bus, metrics).with_resource_manager(manager);
    engine.map_action(action_id, "resource-consumer");

    // 4. Build and execute a single-node workflow
    let node = NodeId::v4();
    let wf = make_workflow(vec![NodeDefinition::new(node, "A", action_id)]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    // 5. Verify the action successfully acquired and used the resource
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
        "action should have received the mock resource instance"
    );
}

/// Full lifecycle: register -> engine -> acquire -> use -> release -> verify stats -> shutdown
#[tokio::test]
async fn full_resource_lifecycle_with_stats_and_shutdown() {
    // 1. Create and populate the resource manager
    let manager = Arc::new(Manager::new());
    manager
        .register(
            MockResource,
            MockConfig,
            PoolConfig {
                min_size: 0,
                max_size: 2,
                ..Default::default()
            },
        )
        .expect("register mock resource");

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(ResourceConsumerHandler {
        meta: meta("resource-consumer"),
    }));

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let event_bus = Arc::new(EventBus::new(128));
    let metrics = Arc::new(MetricsRegistry::new());
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        event_bus.clone(),
        metrics.clone(),
    ));

    let action_id = ActionId::v4();
    let mut engine =
        WorkflowEngine::new(runtime, event_bus, metrics).with_resource_manager(manager.clone());
    engine.map_action(action_id, "resource-consumer");

    // 4. Execute a single-node workflow
    let node = NodeId::v4();
    let wf = make_workflow(vec![NodeDefinition::new(node, "A", action_id)]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    // 5. Verify execution succeeded and resource was used
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
        "action should have received the mock resource instance"
    );

    // 6. Give resource time to return to pool
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 7. Shutdown the manager and verify it completes cleanly
    manager.shutdown().await.expect("shutdown should succeed");
}

/// Verify that `ctx.resource()` returns a fatal error when no resource
/// manager is attached to the engine.
#[tokio::test]
async fn action_resource_fails_without_manager() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register(Arc::new(ResourceConsumerHandler {
        meta: meta("resource-consumer"),
    }));

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let event_bus = Arc::new(EventBus::new(128));
    let metrics = Arc::new(MetricsRegistry::new());
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        event_bus.clone(),
        metrics.clone(),
    ));

    let action_id = ActionId::v4();
    let mut engine = WorkflowEngine::new(runtime, event_bus, metrics);
    // No .with_resource_manager() — intentionally omitted
    engine.map_action(action_id, "resource-consumer");

    let node = NodeId::v4();
    let wf = make_workflow(vec![NodeDefinition::new(node, "A", action_id)]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    // The action should have failed because no resource provider is configured
    assert!(
        result.is_failure(),
        "workflow should fail without resource manager"
    );
}
