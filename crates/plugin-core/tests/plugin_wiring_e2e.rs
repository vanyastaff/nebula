//! End-to-end keystone test: plugin → engine action wiring.
//!
//! ## Test plan
//!
//! `without_plugin_dispatch_fails`
//!   RED witness: dispatching `core.set_fields` on an unwired engine returns
//!   a `RuntimeError::ActionNotFound`-classified error. This test is the
//!   falsifiable proof that the gap is real — removing `with_plugin` must
//!   make the green test below turn red.
//!
//! `with_plugin_set_fields_executes_and_merges`
//!   GREEN proof: after `engine.with_plugin(core_plugin)`, the same
//!   workflow reaches `Completed` and the output contains the exact merged
//!   fields. Asserts concrete output values, not `is_ok()`.
//!
//! `with_plugin_duplicate_key_returns_typed_error`
//!   Unit: calling `with_plugin` twice with the same plugin key returns
//!   `PluginWiringError::DuplicatePlugin`.
//!
//! `with_plugin_duplicate_action_key_returns_typed_error`
//!   Unit: pre-registering an action with the same key as one inside the
//!   plugin returns `PluginWiringError::DuplicateActionKey`.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata, ActionResult,
};
use nebula_engine::ActionExecutor;
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner, PluginWiringError,
    WorkflowEngine,
};
use nebula_execution::context::ExecutionBudget;
use nebula_metrics::MetricsRegistry;
use nebula_plugin::{Plugin, PluginError, PluginManifest, ResolvedPlugin};
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, NodeDefinition, ParamValue, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Engine builder helper ─────────────────────────────────────────────────────

fn make_engine() -> WorkflowEngine {
    let registry = Arc::new(ActionRegistry::new());
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("ActionRuntime must build in tests"),
    );
    WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine must build in tests")
}

fn core_plugin() -> Arc<ResolvedPlugin> {
    Arc::new(
        ResolvedPlugin::from(
            CorePlugin::try_new().expect("CorePlugin::try_new must succeed in tests"),
        )
        .expect("CorePlugin must resolve without namespace errors"),
    )
}

// ── Workflow builder helper ───────────────────────────────────────────────────

fn set_fields_workflow(assignments_json: serde_json::Value) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "SetFields step",
        "core",
        "core.set_fields",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("assignments", ParamValue::literal(assignments_json));

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-set-fields".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![node],
        connections: vec![],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

fn scope() -> nebula_storage_port::Scope {
    nebula_engine::store_seam::single_tenant_scope()
}

// ── RED witness ───────────────────────────────────────────────────────────────

/// Without `with_plugin`, `core.set_fields` is not in the `ActionRegistry`.
/// The engine records the node failure with an action-not-found error and
/// returns `ExecutionStatus::Failed`.
///
/// This test is the falsifiable RED: removing `with_plugin` from the GREEN test
/// below causes it to hit this same failure mode (the execution reaches `Failed`
/// with an action-not-found node error instead of `Completed`).
#[tokio::test]
async fn without_plugin_dispatch_fails() {
    let engine = make_engine(); // no with_plugin call

    let workflow = set_fields_workflow(serde_json::json!([
        {"name": "result", "value": 42}
    ]));

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error — failure is recorded in the result");

    // The engine records action-not-found as a node error and sets status = Failed.
    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Failed,
        "execution without a wired plugin must reach Failed; got {:?}",
        result.status
    );

    // At least one node error must mention "not found".
    let error_texts: Vec<&str> = result.node_errors.values().map(String::as_str).collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must contain an action-not-found message; got: {error_texts:?}"
    );
}

// ── GREEN proof ───────────────────────────────────────────────────────────────

/// After `with_plugin`, `core.set_fields` is executable. The output must
/// contain exactly the merged fields — `is_ok()` alone does not prove correctness.
#[tokio::test]
async fn with_plugin_set_fields_executes_and_merges() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = set_fields_workflow(serde_json::json!([
        {"name": "greeting",  "value": "hello"},
        {"name": "answer",    "value": 42},
        {"name": "flag",      "value": true}
    ]));

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            // The workflow's node input is resolved from `parameters`; `data`
            // defaults to null (empty base object) when not supplied.
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("with_plugin(CorePlugin) + core.set_fields must succeed");

    // The execution must complete.
    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?}",
        result.status
    );

    // The node output must contain the exact merged fields.
    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    assert_eq!(
        node_output["greeting"],
        serde_json::json!("hello"),
        "greeting field must be set"
    );
    assert_eq!(
        node_output["answer"],
        serde_json::json!(42),
        "answer field must be set"
    );
    assert_eq!(
        node_output["flag"],
        serde_json::json!(true),
        "flag field must be set"
    );
}

// ── Duplicate-key unit tests ──────────────────────────────────────────────────

/// Registering the same plugin twice returns `PluginWiringError::DuplicatePlugin`.
#[tokio::test]
async fn with_plugin_duplicate_plugin_key_returns_typed_error() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("first with_plugin must succeed");

    let result = engine.with_plugin(core_plugin());
    assert!(
        result.is_err(),
        "second with_plugin with same key must fail"
    );
    // WorkflowEngine does not implement Debug, so unwrap_err() is unavailable;
    // extract the error via Result::err().
    let err = result.err().expect("checked is_err() above");

    assert!(
        matches!(err, PluginWiringError::DuplicatePlugin { ref plugin_key } if plugin_key.as_str() == "core"),
        "expected DuplicatePlugin{{plugin_key: 'core'}}; got: {err:?}"
    );
}

// ── on_load contract ─────────────────────────────────────────────────────────

/// Minimal `ActionFactory` stub used by `FailOnLoadPlugin`.
///
/// The factory carries an action key under the `"failplugin."` namespace.
/// Its `instantiate` is never called in this test — wiring aborts before
/// registration.
#[derive(Debug)]
struct StubFactory {
    meta: ActionMetadata,
}

impl StubFactory {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                nebula_core::action_key!("failplugin.noop"),
                "Noop",
                "A stub action that is never dispatched",
            ),
        }
    }
}

impl ActionFactory for StubFactory {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _ctx: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        // This method must never be called: on_load fails before registration.
        Box::pin(async { unreachable!("StubFactory::instantiate must not be called in this test") })
    }
}

/// A plugin whose `on_load` always returns an error.
///
/// Used to prove that `with_plugin` honours the `on_load` contract: when the
/// hook fails, no factories are registered and the error is surfaced as
/// `PluginWiringError::OnLoad`.
#[derive(Debug)]
struct FailOnLoadPlugin {
    manifest: PluginManifest,
}

impl FailOnLoadPlugin {
    fn new() -> Self {
        let manifest = PluginManifest::builder("failplugin", "Fail-on-load test plugin")
            .build()
            .expect("FailOnLoadPlugin manifest is a statically valid test fixture");
        Self { manifest }
    }
}

impl Plugin for FailOnLoadPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn ActionFactory>> {
        vec![Arc::new(StubFactory::new())]
    }

    fn on_load(&self) -> Result<(), PluginError> {
        Err(PluginError::NotFound("failplugin".parse().unwrap()))
    }
}

/// `Plugin::on_load` is load-bearing: when it returns `Err`, `with_plugin`
/// must surface `PluginWiringError::OnLoad` and leave the engine state
/// completely unchanged — the plugin's actions must not be dispatchable.
///
/// RED without the `on_load` call: `with_plugin` would succeed and the action
/// would be registered.
#[tokio::test]
async fn with_plugin_on_load_failure_aborts_wiring() {
    let plugin = Arc::new(
        ResolvedPlugin::from(FailOnLoadPlugin::new())
            .expect("FailOnLoadPlugin must pass namespace validation"),
    );

    let engine = make_engine();

    // The wiring must fail with the typed OnLoad variant.
    let result = engine.with_plugin(Arc::clone(&plugin));
    assert!(
        result.is_err(),
        "with_plugin must fail when on_load returns Err"
    );
    let err = result.err().expect("checked is_err() above");

    assert!(
        matches!(
            err,
            PluginWiringError::OnLoad { ref plugin_key, .. }
            if plugin_key.as_str() == "failplugin"
        ),
        "expected OnLoad{{plugin_key: 'failplugin'}}; got: {err:?}"
    );

    // Nothing must be registered: dispatching the action must fail with
    // action-not-found, proving the engine state is unchanged.
    let engine = make_engine(); // fresh engine after the failed wiring attempt
    let action_key_str = "failplugin.noop";
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "Stub step",
        "failplugin",
        action_key_str,
    )
    .expect("NodeDefinition must build");

    let workflow = WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-on-load-abort".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![node],
        connections: vec![],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    let dispatch_result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error");

    assert_eq!(
        dispatch_result.status,
        nebula_execution::ExecutionStatus::Failed,
        "action must be absent from registry after failed on_load; got {:?}",
        dispatch_result.status
    );
    let error_texts: Vec<&str> = dispatch_result
        .node_errors
        .values()
        .map(String::as_str)
        .collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must show action-not-found; got: {error_texts:?}"
    );
}

/// Pre-registering an action with the same key as a plugin action returns
/// `PluginWiringError::DuplicateActionKey`.
#[tokio::test]
async fn with_plugin_duplicate_action_key_returns_typed_error() {
    use nebula_action::action::Action;
    use nebula_plugin_core::actions::set_fields::SetFields;

    // Build an engine whose ActionRegistry already contains core.set_fields.
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(SetFields::metadata(), SetFields);

    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("ActionRuntime must build"),
    );
    let engine = WorkflowEngine::new(runtime, metrics).expect("WorkflowEngine must build");

    let result = engine.with_plugin(core_plugin());
    assert!(
        result.is_err(),
        "with_plugin must fail when action key already registered"
    );
    // WorkflowEngine does not implement Debug, so unwrap_err() is unavailable;
    // extract the error via Result::err().
    let err = result.err().expect("checked is_err() above");

    assert!(
        matches!(
            err,
            PluginWiringError::DuplicateActionKey { ref action_key, .. }
            if action_key.as_str() == "core.set_fields"
        ),
        "expected DuplicateActionKey{{action_key: 'core.set_fields'}}; got: {err:?}"
    );
}
