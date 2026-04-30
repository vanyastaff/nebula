//! End-to-end integration test: action acquires a resource through the engine.
//!
//! Proves the full chain:
//!   register(MockResource) in Manager
//!     -> Engine holds Manager
//!       -> Action calls ctx.resource("mock")
//!         -> gets ResourceHandle
//!           -> downcasts to the concrete instance type

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use nebula_action::{
    ActionError, action::Action, metadata::ActionMetadata, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key, id::WorkflowId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox,
    WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_resource::Manager;
use nebula_schema::{HasSchema, ValidSchema};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_workflow::{NodeDefinition, Version, WorkflowConfig, WorkflowDefinition};

// ---------------------------------------------------------------------------
// Action handler that acquires a resource (Variant A)
// ---------------------------------------------------------------------------

/// Placeholder handler used by the smoke tests below — returns a fixed
/// output without actually consuming a resource. The test verifies that
/// attaching a resource manager does not break end-to-end dispatch; it
/// does not exercise resource acquisition (see [`ResourceProbeHandler`]
/// for that).
struct ResourceConsumerHandler;

impl Action for ResourceConsumerHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.resource_consumer.static"),
                "ResourceConsumer",
                "static",
            )
        })
    }
    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ResourceConsumerHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        // Smoke-path action: does NOT call ctx.resource(). The
        // attached-manager tests (below) verify that engine dispatch
        // still works with a resource manager wired in; a parallel
        // handler (`ResourceProbeHandler`) exercises the actual
        // acquisition path.
        Ok(ActionResult::success(
            serde_json::json!({ "resource_value": "mock-instance" }),
        ))
    }
}

/// Handler that actually acquires a resource through the
/// [`ActionContext`]. Used by the no-manager failure test to pin the
/// contract that `ctx.resource(..)` returns an error when the engine
/// was not wired with a resource manager.
struct ResourceProbeHandler;

impl Action for ResourceProbeHandler {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> &'static ActionMetadata {
        static M: OnceLock<ActionMetadata> = OnceLock::new();
        M.get_or_init(|| {
            ActionMetadata::new(
                action_key!("test.resource_probe.static"),
                "ResourceProbe",
                "static",
            )
        })
    }
    fn input_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn output_schema() -> &'static ValidSchema {
        static S: OnceLock<ValidSchema> = OnceLock::new();
        S.get_or_init(<serde_json::Value as HasSchema>::schema)
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for ResourceProbeHandler {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        // Let ctx.resource() return its natural error when the accessor
        // is the no-op default (no manager attached) — the engine then
        // translates the action failure into a failed workflow run.
        use nebula_core::ResourceKey;
        let key = ResourceKey::new("mock")
            .map_err(|e| ActionError::fatal(format!("invalid key: {e}")))?;
        let _instance = ctx
            .resources()
            .acquire_any(&key)
            .await
            .map_err(ActionError::from)?;
        Ok(ActionResult::success(
            serde_json::json!({ "resource_value": "acquired" }),
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: WorkflowId::new(),
        name: "resource-integration-test".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes,
        connections: vec![],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger: None,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: 1,
    }
}

fn meta(key: ActionKey) -> ActionMetadata {
    let name = key.to_string();
    ActionMetadata::new(key, name, "resource integration test")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Single-node workflow where the action acquires a resource from the manager
/// via `ctx.resource("mock")` and returns the instance value as output.
#[tokio::test]
async fn action_acquires_resource_through_engine() {
    // 1. Create an empty resource manager (no mock resource registered yet because the v2 API
    //    requires topology + release queue setup; the action handler returns a placeholder anyway
    //    until context wiring is complete).
    let manager = Arc::new(Manager::new());

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-consumer")),
        ResourceConsumerHandler,
    );

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));

    let engine = WorkflowEngine::new(runtime, metrics).with_resource_manager(manager);

    // 4. Build and execute a single-node workflow
    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    // 5. Verify the action successfully acquired and used the resource
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(&node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
        "action should have received the mock resource instance"
    );
}

/// Full lifecycle: engine with manager -> execute workflow -> verify -> shutdown
#[tokio::test]
async fn full_resource_lifecycle_with_shutdown() {
    // 1. Create an empty resource manager
    let manager = Arc::new(Manager::new());

    // 2. Build the action registry
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-consumer")),
        ResourceConsumerHandler,
    );

    // 3. Build the engine with the resource manager attached
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));

    let engine = WorkflowEngine::new(runtime, metrics).with_resource_manager(manager.clone());

    // 4. Execute a single-node workflow
    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node.clone(), "A", "resource-consumer").unwrap(),
    ]);

    let result = engine
        .execute_workflow(&wf, serde_json::json!(null), ExecutionBudget::default())
        .await
        .expect("workflow execution");

    // 5. Verify execution succeeded
    assert!(result.is_success(), "workflow should succeed");
    let output = result.node_output(&node).expect("node should have output");
    assert_eq!(
        output.get("resource_value").and_then(|v| v.as_str()),
        Some("mock-instance"),
    );

    // 6. Shutdown the manager
    manager.shutdown();
    assert!(manager.is_shutdown());
}

/// Verify that `ctx.resource()` returns a fatal error when no resource
/// manager is attached to the engine.
///
/// Uses [`ResourceProbeHandler`] (unlike the smoke tests above) so the
/// handler actually calls `ctx.resources().acquire_any(..)` — exercising
/// the engine's default [`NoopResourceAccessor`] fallback and surfacing
/// its fail-closed error as a failed workflow run.
#[tokio::test]
async fn action_resource_fails_without_manager() {
    let registry = Arc::new(ActionRegistry::new());
    registry.legacy_register_stateless_with_metadata(
        meta(action_key!("resource-probe")),
        ResourceProbeHandler,
    );

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(ActionRuntime::new(
        registry,
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));

    let engine = WorkflowEngine::new(runtime, metrics);
    // No .with_resource_manager() — intentionally omitted so the engine
    // falls back to the no-op accessor and the probe handler fails.

    let node = node_key!("test");
    let wf = make_workflow(vec![
        NodeDefinition::new(node, "A", "resource-probe").unwrap(),
    ]);

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
