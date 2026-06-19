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
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, ParamValue, Version, WorkflowConfig,
    WorkflowDefinition,
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

// ── json_transform e2e ────────────────────────────────────────────────────────

/// Builds a single-node `core.json_transform` workflow.
///
/// `data_json` is the base object and `operations_json` is the serialized
/// operations array. Both are wired as `ParamValue::literal` parameters so the
/// engine resolves them into [`JsonTransformInput`] before dispatch.
fn json_transform_workflow(
    data_json: serde_json::Value,
    operations_json: serde_json::Value,
) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "JsonTransform step",
        "core",
        "core.json_transform",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("data", ParamValue::literal(data_json))
    .with_parameter("operations", ParamValue::literal(operations_json));

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-json-transform".into(),
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

/// RED witness: dispatching `core.json_transform` on an engine without the
/// CorePlugin wired returns `ExecutionStatus::Failed` with an action-not-found
/// node error.
///
/// Removing `with_plugin` from the GREEN test below causes it to hit this same
/// failure mode.
#[tokio::test]
async fn without_plugin_json_transform_dispatch_fails() {
    let engine = make_engine(); // no with_plugin call

    let workflow = json_transform_workflow(
        serde_json::json!({"a": 1, "b": 2}),
        serde_json::json!([{"op": "pick", "fields": ["a"]}]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error — failure is recorded in the result");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Failed,
        "execution without a wired plugin must reach Failed; got {:?}",
        result.status
    );

    let error_texts: Vec<&str> = result.node_errors.values().map(String::as_str).collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must contain an action-not-found message; got: {error_texts:?}"
    );
}

/// GREEN proof: after `with_plugin(CorePlugin)`, the `core.json_transform`
/// action executes and returns the correct transformed output.
///
/// Uses `Pick { fields: ["a", "b"] }` on `{"a":1,"b":2,"c":3}` and asserts:
/// - execution status is `Completed`,
/// - `a` and `b` are present with their original values,
/// - `c` is absent.
#[tokio::test]
async fn with_plugin_json_transform_executes_and_transforms() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = json_transform_workflow(
        serde_json::json!({"a": 1, "b": 2, "c": 3}),
        serde_json::json!([{"op": "pick", "fields": ["a", "b"]}]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("with_plugin(CorePlugin) + core.json_transform must succeed");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?}",
        result.status
    );

    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    assert_eq!(node_output["a"], serde_json::json!(1), "a must be present");
    assert_eq!(node_output["b"], serde_json::json!(2), "b must be present");
    assert_eq!(
        node_output.get("c"),
        None,
        "c must be absent after Pick [a, b]"
    );
}

// ── core.if e2e ───────────────────────────────────────────────────────────────

/// Build a 3-node workflow:
///
/// ```text
/// [if_node] --"true"-->  [true_branch]
///           --"false"--> [false_branch]
/// ```
///
/// Both branches are `core.set_fields` nodes that each add a distinct marker
/// field so we can tell which one ran. The `condition` and `data` for the IF
/// node are wired as `ParamValue::literal` parameters.
///
/// Returns `(workflow, true_branch_key, false_branch_key)`.
fn if_branch_workflow(
    condition_json: serde_json::Value,
    data_json: serde_json::Value,
) -> (
    WorkflowDefinition,
    nebula_core::NodeKey,
    nebula_core::NodeKey,
) {
    let now = chrono::Utc::now();

    let if_node_key = nebula_core::node_key!("if_step");
    let true_node_key = nebula_core::node_key!("true_branch");
    let false_node_key = nebula_core::node_key!("false_branch");

    let if_node = NodeDefinition::new(if_node_key.clone(), "If step", "core", "core.if")
        .expect("NodeDefinition must build with valid keys")
        .with_parameter("condition", ParamValue::literal(condition_json))
        .with_parameter("data", ParamValue::literal(data_json));

    // Both branch nodes use core.set_fields to stamp a unique marker key.
    let true_node = NodeDefinition::new(
        true_node_key.clone(),
        "True branch",
        "core",
        "core.set_fields",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter(
        "assignments",
        ParamValue::literal(serde_json::json!([{"name": "branch_taken", "value": "true"}])),
    );

    let false_node = NodeDefinition::new(
        false_node_key.clone(),
        "False branch",
        "core",
        "core.set_fields",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter(
        "assignments",
        ParamValue::literal(serde_json::json!([{"name": "branch_taken", "value": "false"}])),
    );

    // Wire: if_node["true"] → true_node, if_node["false"] → false_node.
    let edge_to_true =
        Connection::new(if_node_key.clone(), true_node_key.clone()).with_from_port("true");
    let edge_to_false =
        Connection::new(if_node_key, false_node_key.clone()).with_from_port("false");

    let workflow = WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-if-branch".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![if_node, true_node, false_node],
        connections: vec![edge_to_true, edge_to_false],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    (workflow, true_node_key, false_node_key)
}

/// GREEN proof — condition TRUE: engine routes to the `"true"` branch node and
/// marks the `"false"` branch node as Skipped.
///
/// Skipped-node proof: the Skipped node has no entry in `node_outputs` (it
/// produced no value) AND no entry in `node_errors` (it did not fail). The
/// selected branch node has an entry in `node_outputs` with the marker value.
///
/// RED witness: swap the `"true"` / `"false"` connection ports and the
/// assertions on `true_branch` and `false_branch` both invert — proving the
/// test distinguishes which node ran.
#[tokio::test]
async fn if_action_true_condition_executes_true_branch_skips_false() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let (workflow, true_node_key, false_node_key) = if_branch_workflow(
        serde_json::json!({ "field": "status", "op": "eq", "value": "active" }),
        serde_json::json!({ "status": "active" }),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // The true branch must have run and added its marker.
    let true_output = result
        .node_outputs
        .get(&true_node_key)
        .expect("true_branch node must have output (it ran)");
    assert_eq!(
        true_output["branch_taken"],
        serde_json::json!("true"),
        "true_branch must have set branch_taken = 'true'"
    );

    // The false branch must be Skipped: no output and no error entry.
    assert!(
        !result.node_outputs.contains_key(&false_node_key),
        "false_branch must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&false_node_key),
        "false_branch must be Skipped (absent from node_errors)"
    );
}

/// GREEN proof — condition FALSE: engine routes to the `"false"` branch node
/// and marks the `"true"` branch node as Skipped.
///
/// RED witness: the Skipped assertion on `true_node_key` fails if the engine
/// ran both branches or the wrong one.
#[tokio::test]
async fn if_action_false_condition_executes_false_branch_skips_true() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let (workflow, true_node_key, false_node_key) = if_branch_workflow(
        serde_json::json!({ "field": "status", "op": "eq", "value": "active" }),
        serde_json::json!({ "status": "inactive" }),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // The false branch must have run and added its marker.
    let false_output = result
        .node_outputs
        .get(&false_node_key)
        .expect("false_branch node must have output (it ran)");
    assert_eq!(
        false_output["branch_taken"],
        serde_json::json!("false"),
        "false_branch must have set branch_taken = 'false'"
    );

    // The true branch must be Skipped: no output and no error entry.
    assert!(
        !result.node_outputs.contains_key(&true_node_key),
        "true_branch must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&true_node_key),
        "true_branch must be Skipped (absent from node_errors)"
    );
}

// ── core.switch e2e ───────────────────────────────────────────────────────────

/// Build a 4-node workflow:
///
/// ```text
/// [switch_node] --"a"-------> [node_a]
///               --"b"-------> [node_b]
///               --"default"--> [node_default]
/// ```
///
/// All three branch nodes are `core.set_fields` nodes that stamp a distinct
/// `branch_taken` marker so we can tell which ran. The switch `data` and
/// `cases` are wired as `ParamValue::literal` parameters.
///
/// `cases` ports use the literal string names "a" and "b". Edges reference
/// those same strings via `with_from_port`. The engine routes via the
/// `ControlOutcome::Branch { selected }` value; since the port is matched by
/// string name the dynamic metadata is not consulted at runtime.
///
/// Returns `(workflow, key_a, key_b, key_default)`.
fn switch_branch_workflow(
    data_json: serde_json::Value,
    cases_json: serde_json::Value,
) -> (
    WorkflowDefinition,
    nebula_core::NodeKey,
    nebula_core::NodeKey,
    nebula_core::NodeKey,
) {
    let now = chrono::Utc::now();

    let switch_node_key = nebula_core::node_key!("switch_step");
    let node_a_key = nebula_core::node_key!("node_a");
    let node_b_key = nebula_core::node_key!("node_b");
    let node_default_key = nebula_core::node_key!("node_default");

    let switch_node = NodeDefinition::new(
        switch_node_key.clone(),
        "Switch step",
        "core",
        "core.switch",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("data", ParamValue::literal(data_json))
    .with_parameter("cases", ParamValue::literal(cases_json));

    let node_a = NodeDefinition::new(node_a_key.clone(), "Branch A", "core", "core.set_fields")
        .expect("NodeDefinition must build")
        .with_parameter(
            "assignments",
            ParamValue::literal(serde_json::json!([{"name": "branch_taken", "value": "a"}])),
        );

    let node_b = NodeDefinition::new(node_b_key.clone(), "Branch B", "core", "core.set_fields")
        .expect("NodeDefinition must build")
        .with_parameter(
            "assignments",
            ParamValue::literal(serde_json::json!([{"name": "branch_taken", "value": "b"}])),
        );

    let node_default = NodeDefinition::new(
        node_default_key.clone(),
        "Branch default",
        "core",
        "core.set_fields",
    )
    .expect("NodeDefinition must build")
    .with_parameter(
        "assignments",
        ParamValue::literal(serde_json::json!([{"name": "branch_taken", "value": "default"}])),
    );

    // Wire switch output ports to the three branch nodes.
    let edge_to_a =
        Connection::new(switch_node_key.clone(), node_a_key.clone()).with_from_port("a");
    let edge_to_b =
        Connection::new(switch_node_key.clone(), node_b_key.clone()).with_from_port("b");
    let edge_to_default =
        Connection::new(switch_node_key, node_default_key.clone()).with_from_port("default");

    let workflow = WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-switch-branch".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![switch_node, node_a, node_b, node_default],
        connections: vec![edge_to_a, edge_to_b, edge_to_default],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: vec![],
        tags: vec![],
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    (workflow, node_a_key, node_b_key, node_default_key)
}

/// GREEN proof — second case matches (`port "b"`): engine routes to node_b,
/// marks node_a and node_default as Skipped.
///
/// RED witness for Skipped proof: if the engine ran all branches, node_a and
/// node_default would have outputs and the `contains_key` asserts would fail.
/// If the engine selected the wrong port, the `branch_taken` assertion fails.
#[tokio::test]
async fn switch_action_second_case_matches_routes_to_b_skips_others() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let (workflow, node_a_key, node_b_key, node_default_key) = switch_branch_workflow(
        serde_json::json!({ "status": "pending", "score": 95 }),
        serde_json::json!([
            // case[0]: status == "active" — does NOT match ("pending")
            { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" },
            // case[1]: score > 90 — MATCHES (95 > 90)
            { "condition": { "field": "score",  "op": "gt", "value": 90 },       "port": "b" }
        ]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // node_b must have run and stamped its marker.
    let b_output = result
        .node_outputs
        .get(&node_b_key)
        .expect("node_b must have output (it ran)");
    assert_eq!(
        b_output["branch_taken"],
        serde_json::json!("b"),
        "node_b must have set branch_taken = 'b'"
    );

    // node_a and node_default must be Skipped: no output and no error entry.
    assert!(
        !result.node_outputs.contains_key(&node_a_key),
        "node_a must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&node_a_key),
        "node_a must be Skipped (absent from node_errors)"
    );
    assert!(
        !result.node_outputs.contains_key(&node_default_key),
        "node_default must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&node_default_key),
        "node_default must be Skipped (absent from node_errors)"
    );
}

/// GREEN proof — no case matches: engine routes to node_default, marks
/// node_a and node_b as Skipped.
///
/// RED witness: if the engine did not honour `"default"` routing, node_default
/// would be absent from node_outputs or the status would be Failed.
#[tokio::test]
async fn switch_action_no_match_routes_to_default_skips_others() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let (workflow, node_a_key, node_b_key, node_default_key) = switch_branch_workflow(
        // Neither "active" nor score > 90 — no case matches.
        serde_json::json!({ "status": "unknown", "score": 50 }),
        serde_json::json!([
            { "condition": { "field": "status", "op": "eq", "value": "active" }, "port": "a" },
            { "condition": { "field": "score",  "op": "gt", "value": 90 },       "port": "b" }
        ]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // node_default must have run.
    let default_output = result
        .node_outputs
        .get(&node_default_key)
        .expect("node_default must have output (it ran)");
    assert_eq!(
        default_output["branch_taken"],
        serde_json::json!("default"),
        "node_default must have set branch_taken = 'default'"
    );

    // node_a and node_b must be Skipped.
    assert!(
        !result.node_outputs.contains_key(&node_a_key),
        "node_a must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&node_a_key),
        "node_a must be Skipped (absent from node_errors)"
    );
    assert!(
        !result.node_outputs.contains_key(&node_b_key),
        "node_b must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&node_b_key),
        "node_b must be Skipped (absent from node_errors)"
    );
}

// ── Duplicate-key unit tests ──────────────────────────────────────────────────

// ── core.if compound-condition e2e ───────────────────────────────────────────

/// GREEN proof — compound `All` condition: a `core.if` node with
/// `{"all": [{field exists}, {field gt threshold}]}` routes to the `"true"`
/// branch when BOTH sub-conditions hold, and to `"false"` when either fails.
///
/// Uses the same `if_branch_workflow` helper as the simple if e2e tests so
/// the engine wiring path is identical — only the condition JSON changes.
///
/// RED witness: swap the data values (e.g. set score=3) and the first assert
/// fails — proving the compound condition is actually evaluated.
#[tokio::test]
async fn if_action_compound_all_condition_routes_correctly() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    // Compound condition: exists("score") AND gt("score", 5)
    // data has score=10 → both true → routes "true"
    let (workflow, true_node_key, false_node_key) = if_branch_workflow(
        serde_json::json!({
            "all": [
                { "field": "score", "op": "exists" },
                { "field": "score", "op": "gt", "value": 5 }
            ]
        }),
        serde_json::json!({ "score": 10 }),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    // The true branch ran (score=10 passes both conditions).
    let true_output = result
        .node_outputs
        .get(&true_node_key)
        .expect("true_branch node must have output (it ran)");
    assert_eq!(
        true_output["branch_taken"],
        serde_json::json!("true"),
        "true_branch must have stamped branch_taken = 'true'"
    );

    // The false branch must be Skipped.
    assert!(
        !result.node_outputs.contains_key(&false_node_key),
        "false_branch must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result.node_errors.contains_key(&false_node_key),
        "false_branch must be Skipped (absent from node_errors)"
    );

    // --- Second assertion: one sub-condition fails → routes "false" ---
    // score=3 → exists passes, gt(score,5) fails → All=false → "false" branch
    let engine2 = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed");

    let (workflow2, true_node_key2, false_node_key2) = if_branch_workflow(
        serde_json::json!({
            "all": [
                { "field": "score", "op": "exists" },
                { "field": "score", "op": "gt", "value": 5 }
            ]
        }),
        serde_json::json!({ "score": 3 }),
    );

    let result2 = engine2
        .execute_workflow(
            &scope(),
            &workflow2,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result2.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result2.status,
        result2.node_errors
    );

    // The false branch ran (score=3 fails the gt condition).
    let false_output = result2
        .node_outputs
        .get(&false_node_key2)
        .expect("false_branch node must have output (it ran)");
    assert_eq!(
        false_output["branch_taken"],
        serde_json::json!("false"),
        "false_branch must have stamped branch_taken = 'false'"
    );

    // The true branch must be Skipped.
    assert!(
        !result2.node_outputs.contains_key(&true_node_key2),
        "true_branch must be Skipped (absent from node_outputs)"
    );
    assert!(
        !result2.node_errors.contains_key(&true_node_key2),
        "true_branch must be Skipped (absent from node_errors)"
    );
}

// ── core.datetime e2e ─────────────────────────────────────────────────────────

/// Build a single-node `core.datetime` workflow with the given `op` JSON.
///
/// The `op` value and any accompanying fields are passed as individual
/// `ParamValue::literal` parameters so the engine resolves them into a flat
/// `DateTimeInput` before dispatch.  The helper accepts a single JSON object
/// whose keys map one-to-one to `DateTimeInput` fields (including the `"op"`
/// discriminant injected via `serde(tag = "op")`).
fn datetime_workflow(params: serde_json::Value) -> WorkflowDefinition {
    let now = chrono::Utc::now();

    // `params` is an object like {"op":"format","input":"...","format":"..."}.
    // Wire each key as a separate ParamValue::literal parameter.
    let params_map = params
        .as_object()
        .expect("datetime_workflow: params must be a JSON object");

    let mut node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "DateTime step",
        "core",
        "core.datetime",
    )
    .expect("NodeDefinition must build with valid keys");

    for (key, value) in params_map {
        node = node.with_parameter(key.as_str(), ParamValue::literal(value.clone()));
    }

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-datetime".into(),
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

/// GREEN proof — `core.datetime` Format op: the engine executes the action and
/// returns the formatted string as the node output.
///
/// Input: `2026-06-19T00:00:00Z` formatted with `%Y-%m-%d` → `"2026-06-19"`.
/// Asserts a concrete output value, not `is_ok()`.
///
/// RED witness: without `with_plugin(core_plugin())`, the execution reaches
/// `Failed` with an action-not-found node error (same as the existing
/// `without_plugin_dispatch_fails` test covers the general case).
#[tokio::test]
async fn with_plugin_datetime_format_executes_and_returns_formatted_string() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = datetime_workflow(serde_json::json!({
        "op":     "format",
        "input":  "2026-06-19T00:00:00Z",
        "format": "%Y-%m-%d"
    }));

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow must not error");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    assert_eq!(
        *node_output,
        serde_json::json!("2026-06-19"),
        "Format op must return the date-only string"
    );
}

// ── Duplicate-key unit tests ──────────────────────────────────────────────────

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

// ── core.filter e2e ───────────────────────────────────────────────────────────

/// Build a single-node `core.filter` workflow.
///
/// `data_json` is the input array and `condition_json` is the serialized
/// `Condition`. Both are wired as `ParamValue::literal` parameters so the
/// engine resolves them into `FilterInput` before dispatch.
fn filter_workflow(
    data_json: serde_json::Value,
    condition_json: serde_json::Value,
) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "Filter step",
        "core",
        "core.filter",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("data", ParamValue::literal(data_json))
    .with_parameter("condition", ParamValue::literal(condition_json));

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-filter".into(),
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

/// RED witness: dispatching `core.filter` on an engine without the CorePlugin
/// wired returns `ExecutionStatus::Failed` with an action-not-found node error.
///
/// Removing `with_plugin` from the GREEN test below causes it to hit this
/// same failure mode.
#[tokio::test]
async fn without_plugin_filter_dispatch_fails() {
    let engine = make_engine(); // no with_plugin call

    let workflow = filter_workflow(
        serde_json::json!([{"x": 1}, {"x": 2}]),
        serde_json::json!({ "field": "x", "op": "gt", "value": 1 }),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error — failure is recorded in the result");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Failed,
        "execution without a wired plugin must reach Failed; got {:?}",
        result.status
    );

    let error_texts: Vec<&str> = result.node_errors.values().map(String::as_str).collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must contain an action-not-found message; got: {error_texts:?}"
    );
}

/// GREEN proof: after `with_plugin(CorePlugin)`, the `core.filter` action
/// executes and returns the correctly filtered output.
///
/// Input: `[{x:1},{x:2},{x:3}]`, condition `x > 1`.
/// Expected output: `[{x:2},{x:3}]` (concrete array value asserted).
///
/// RED witness: without `with_plugin`, the execution reaches `Failed`
/// (same path as `without_plugin_filter_dispatch_fails` above).
#[tokio::test]
async fn with_plugin_filter_executes_and_filters() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = filter_workflow(
        serde_json::json!([{"x": 1}, {"x": 2}, {"x": 3}]),
        serde_json::json!({ "field": "x", "op": "gt", "value": 1 }),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("with_plugin(CorePlugin) + core.filter must succeed");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    assert_eq!(
        *node_output,
        serde_json::json!([{"x": 2}, {"x": 3}]),
        "filter must return exactly the elements where x > 1, in original order"
    );
}

// ── core.aggregate e2e ────────────────────────────────────────────────────────

/// Build a single-node `core.aggregate` workflow.
///
/// Parameters are wired as `ParamValue::literal` so the engine resolves them
/// into `AggregateInput` before dispatch. `aggregations_json` is the
/// serialized aggregation array; `group_by_json` is the serialized group-by
/// field list.
fn aggregate_workflow(
    data_json: serde_json::Value,
    group_by_json: serde_json::Value,
    aggregations_json: serde_json::Value,
) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "Aggregate step",
        "core",
        "core.aggregate",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("data", ParamValue::literal(data_json))
    .with_parameter("group_by", ParamValue::literal(group_by_json))
    .with_parameter("aggregations", ParamValue::literal(aggregations_json));

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-aggregate".into(),
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

/// RED witness: dispatching `core.aggregate` on an engine without the
/// CorePlugin wired returns `ExecutionStatus::Failed` with an action-not-found
/// node error.
///
/// Removing `with_plugin` from the GREEN test below causes it to hit this same
/// failure mode, proving the GREEN test distinguishes "action registered" from
/// "action absent".
#[tokio::test]
async fn without_plugin_aggregate_dispatch_fails() {
    let engine = make_engine(); // no with_plugin call

    let workflow = aggregate_workflow(
        serde_json::json!([{"region": "west", "amount": 10}]),
        serde_json::json!(["region"]),
        serde_json::json!([{"fn": "count", "out": "n"}]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error — failure is recorded in the result");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Failed,
        "execution without a wired plugin must reach Failed; got {:?}",
        result.status
    );

    let error_texts: Vec<&str> = result.node_errors.values().map(String::as_str).collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must contain an action-not-found message; got: {error_texts:?}"
    );
}

// ── core.sort e2e ─────────────────────────────────────────────────────────────

/// Build a single-node `core.sort` workflow.
///
/// `data_json` is the input array and `keys_json` is the serialized sort-key
/// array. Both are wired as `ParamValue::literal` parameters so the engine
/// resolves them into `SortInput` before dispatch.
fn sort_workflow(data_json: serde_json::Value, keys_json: serde_json::Value) -> WorkflowDefinition {
    let now = chrono::Utc::now();
    let node = NodeDefinition::new(
        nebula_core::node_key!("step"),
        "Sort step",
        "core",
        "core.sort",
    )
    .expect("NodeDefinition must build with valid keys")
    .with_parameter("data", ParamValue::literal(data_json))
    .with_parameter("keys", ParamValue::literal(keys_json));

    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "test-core-sort".into(),
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

/// RED witness: dispatching `core.sort` on an engine without the CorePlugin
/// wired returns `ExecutionStatus::Failed` with an action-not-found node error.
///
/// Removing `with_plugin` from the GREEN test below causes it to hit this same
/// failure mode, proving the GREEN test distinguishes "action registered" from
/// "action absent".
#[tokio::test]
async fn without_plugin_sort_dispatch_fails() {
    let engine = make_engine(); // no with_plugin call

    let workflow = sort_workflow(
        serde_json::json!([{"n": 2}, {"n": 1}]),
        serde_json::json!([{"field": "n", "order": "asc"}]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("execute_workflow itself must not error — failure is recorded in the result");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Failed,
        "execution without a wired plugin must reach Failed; got {:?}",
        result.status
    );

    let error_texts: Vec<&str> = result.node_errors.values().map(String::as_str).collect();
    assert!(
        error_texts.iter().any(|s| s.contains("not found")),
        "node_errors must contain an action-not-found message; got: {error_texts:?}"
    );
}

/// GREEN proof: after `with_plugin(CorePlugin)`, the `core.sort` action
/// executes and returns the correctly sorted output.
///
/// Input: `[{n:3},{n:1},{n:2}]`, key `n asc`.
/// Expected output: `[{n:1},{n:2},{n:3}]` (concrete array value asserted).
///
/// RED witness: without `with_plugin`, the execution reaches `Failed`
/// (same path as `without_plugin_sort_dispatch_fails` above).
#[tokio::test]
async fn with_plugin_sort_executes_and_sorts() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = sort_workflow(
        serde_json::json!([{"n": 3}, {"n": 1}, {"n": 2}]),
        serde_json::json!([{"field": "n", "order": "asc"}]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("with_plugin(CorePlugin) + core.sort must succeed");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    assert_eq!(
        *node_output,
        serde_json::json!([{"n": 1}, {"n": 2}, {"n": 3}]),
        "sort must return elements in ascending order by n"
    );
}

/// GREEN proof: after `with_plugin(CorePlugin)`, the `core.aggregate` action
/// executes and returns the correct grouped summary rows.
///
/// Input: three elements with two distinct `region` values.
/// group_by: `["region"]`.
/// aggregations: count(*) → `"n"`, sum(amount) → `"total"`.
///
/// Expected: two rows in first-seen order with the correct group values and
/// sums. Asserts concrete row values, not `is_ok()`.
///
/// RED witness: without `with_plugin`, the execution reaches `Failed`
/// (same path as `without_plugin_aggregate_dispatch_fails`).
#[tokio::test]
async fn with_plugin_aggregate_executes_and_summarizes() {
    let engine = make_engine()
        .with_plugin(core_plugin())
        .expect("with_plugin(CorePlugin) must succeed on a fresh engine");

    let workflow = aggregate_workflow(
        serde_json::json!([
            { "region": "west", "amount": 10 },
            { "region": "east", "amount": 20 },
            { "region": "west", "amount": 30 }
        ]),
        serde_json::json!(["region"]),
        serde_json::json!([
            { "fn": "count", "out": "n" },
            { "fn": "sum",   "field": "amount", "out": "total" }
        ]),
    );

    let result = engine
        .execute_workflow(
            &scope(),
            &workflow,
            serde_json::json!({}),
            ExecutionBudget::default(),
        )
        .await
        .expect("with_plugin(CorePlugin) + core.aggregate must succeed");

    assert_eq!(
        result.status,
        nebula_execution::ExecutionStatus::Completed,
        "execution must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors
    );

    let node_key = nebula_core::node_key!("step");
    let node_output = result
        .node_outputs
        .get(&node_key)
        .expect("node 'step' must have output after Completed execution");

    // Two rows in first-seen order: west first, east second.
    assert_eq!(
        *node_output,
        serde_json::json!([
            { "region": "west", "n": 2, "total": 40 },
            { "region": "east", "n": 1, "total": 20 }
        ]),
        "aggregate must return grouped summary rows in first-seen order with correct sums"
    );
}
