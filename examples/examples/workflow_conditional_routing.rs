//! End-to-end workflow example: **conditional routing** with `core.if`, executed
//! through the real `WorkflowEngine`.
//!
//! Every other workflow example is linear (`data_pipeline`, `delay_resume`,
//! `datetime_schedule`). This one branches: a single `core.if` node routes the
//! execution down one of two ports based on a condition, and the engine *skips*
//! the branch that was not selected. The same workflow is run twice — once with
//! input that takes the `true` port, once the `false` port — to show both routes
//! deterministically.
//!
//! It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs` (the `core.if` e2e) and reused
//! by the sibling workflow examples:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## The workflow (order triage)
//!
//! ```text
//!   workflow input: an order { id, amount, customer }
//!        │
//!        ▼
//!   [triage]  core.if   condition: amount >= 1000
//!        │
//!        ├──"true"──▶ [priority]  core.set_fields  → tier="priority", needs_review=true
//!        └──"false"─▶ [standard]  core.set_fields  → tier="standard"
//! ```
//!
//! The `core.if` node passes its `data` (the order) down the selected port; the
//! branch `core.set_fields` node *merges* its assignments onto that order, so the
//! branch output carries the original fields plus the triage stamps. The branch
//! that was not selected is **Skipped**: it appears in neither `node_outputs`
//! (it produced nothing) nor `node_errors` (it did not fail).
//!
//! Each run asserts the selected branch ran with the right stamps *and* that the
//! other branch was skipped, so a routing regression (wrong port, both branches
//! firing, or a skipped-but-errored node) fails loudly.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_conditional_routing
//! ```

use std::{collections::HashMap, sync::Arc};

use anyhow::Context as _;
use nebula_action::ActionResult;
use nebula_engine::ResolvedPlugin;
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
    WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, ParamValue, Version, WorkflowConfig,
    WorkflowDefinition,
};
use serde_json::{Value, json};

/// Orders at or above this amount are routed to priority handling.
const PRIORITY_THRESHOLD: i64 = 1000;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_routing_workflow();

    let priority_key = nebula_core::node_key!("priority");
    let standard_key = nebula_core::node_key!("standard");

    println!("=== core.if conditional routing (threshold: amount >= {PRIORITY_THRESHOLD}) ===\n");

    // ── Run 1: a high-value order takes the `true` port → priority branch ───────
    let high_value = json!({ "id": "A-1042", "amount": 1200, "customer": "acme" });
    println!(
        "Run 1 — order {}: amount 1200 (>= threshold)",
        high_value["id"]
    );
    let run1 = run_once(&engine, &workflow, &high_value).await?;

    let priority_out = run1
        .node_outputs
        .get(&priority_key)
        .context("the `priority` branch must have run for a high-value order")?;
    anyhow::ensure!(
        priority_out["tier"] == json!("priority") && priority_out["needs_review"] == json!(true),
        "priority branch must stamp tier=priority + needs_review=true; got {priority_out}",
    );
    // The original order survives the branch (set_fields merges onto the routed data).
    anyhow::ensure!(
        priority_out["id"] == json!("A-1042") && priority_out["amount"] == json!(1200),
        "priority branch must preserve the routed order fields; got {priority_out}",
    );
    ensure_skipped(&run1, &standard_key, "standard")?;
    println!("  → routed to `priority`; `standard` skipped. Output: {priority_out}\n");

    // ── Run 2: a low-value order takes the `false` port → standard branch ───────
    let standard_order = json!({ "id": "A-2099", "amount": 80, "customer": "beta" });
    println!(
        "Run 2 — order {}: amount 80 (< threshold)",
        standard_order["id"]
    );
    let run2 = run_once(&engine, &workflow, &standard_order).await?;

    let standard_out = run2
        .node_outputs
        .get(&standard_key)
        .context("the `standard` branch must have run for a low-value order")?;
    anyhow::ensure!(
        standard_out["tier"] == json!("standard"),
        "standard branch must stamp tier=standard; got {standard_out}",
    );
    // Standard handling does not flag review — proves the branches differ.
    anyhow::ensure!(
        standard_out.get("needs_review").is_none(),
        "standard branch must NOT set needs_review; got {standard_out}",
    );
    anyhow::ensure!(
        standard_out["id"] == json!("A-2099"),
        "standard branch must preserve the routed order fields; got {standard_out}",
    );
    ensure_skipped(&run2, &priority_key, "priority")?;
    println!("  → routed to `standard`; `priority` skipped. Output: {standard_out}\n");

    println!("Both routes verified: the engine selects one port and skips the other.");
    Ok(())
}

/// Execute the workflow once with `input`, asserting it reaches `Completed`.
async fn run_once(
    engine: &WorkflowEngine,
    workflow: &WorkflowDefinition,
    input: &Value,
) -> anyhow::Result<nebula_engine::ExecutionResult> {
    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            workflow,
            input.clone(),
            ExecutionBudget::default(),
        )
        .await
        .context("executing the conditional-routing workflow")?;

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "workflow must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );
    Ok(result)
}

/// Assert a node was Skipped: it produced no output AND recorded no error.
fn ensure_skipped(
    result: &nebula_engine::ExecutionResult,
    key: &nebula_core::NodeKey,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !result.node_outputs.contains_key(key),
        "`{label}` must be Skipped (no output), but it produced one",
    );
    anyhow::ensure!(
        !result.node_errors.contains_key(key),
        "`{label}` must be Skipped (no error), but it recorded one",
    );
    Ok(())
}

/// Initialise a simple `fmt` tracing subscriber so the `core.*` actions'
/// instrumentation is visible when `RUST_LOG` is set (e.g. `RUST_LOG=info`).
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .try_init();
}

/// Build a standalone `WorkflowEngine` with the first-party `CorePlugin` wired.
///
/// Mirrors `workflow_data_pipeline`'s `build_engine`: the `ActionExecutor` is the
/// identity executor used by the in-process runner; the `core.*` actions
/// themselves are registered by `with_plugin(CorePlugin)`.
fn build_engine() -> anyhow::Result<WorkflowEngine> {
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
        .context("ActionRuntime::try_new")?,
    );
    let engine = WorkflowEngine::new(runtime, metrics).context("WorkflowEngine::new")?;

    let core_plugin = Arc::new(
        ResolvedPlugin::from(CorePlugin::try_new().context("CorePlugin::try_new")?)
            .context("resolving CorePlugin")?,
    );
    engine
        .with_plugin(core_plugin)
        .context("wiring CorePlugin into the engine")
}

/// Build the `triage -> {priority | standard}` routing workflow.
///
/// - `triage` is the entry `core.if` node: it reads the order from the workflow
///   input (`$input`) and evaluates `amount >= PRIORITY_THRESHOLD`.
/// - The `"true"` port routes to `priority` (`core.set_fields`), the `"false"`
///   port to `standard`. Each merges its triage stamps onto the routed order.
fn build_routing_workflow() -> WorkflowDefinition {
    let triage_key = nebula_core::node_key!("triage");
    let priority_key = nebula_core::node_key!("priority");
    let standard_key = nebula_core::node_key!("standard");

    // Entry node: route on the order amount. `data` is the workflow input order.
    let triage_node =
        NodeDefinition::new(triage_key.clone(), "Triage by amount", "core", "core.if")
            .expect("triage NodeDefinition has valid keys")
            .with_parameter("data", ParamValue::expression("$input"))
            .with_parameter(
                "condition",
                ParamValue::literal(
                    json!({ "field": "amount", "op": "gte", "value": PRIORITY_THRESHOLD }),
                ),
            );

    // True port: priority handling — merge tier + a review flag onto the order
    // that `triage` routed through (`core.if` emits its `data` on the selected
    // port; `set_fields` merges its assignments onto that base).
    let priority_node = NodeDefinition::new(
        priority_key.clone(),
        "Priority handling",
        "core",
        "core.set_fields",
    )
    .expect("priority NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(triage_key.clone(), ""))
    .with_parameter(
        "assignments",
        ParamValue::literal(json!([
            { "name": "tier", "value": "priority" },
            { "name": "needs_review", "value": true },
        ])),
    );

    // False port: standard handling — merge tier only onto the routed order.
    let standard_node = NodeDefinition::new(
        standard_key.clone(),
        "Standard handling",
        "core",
        "core.set_fields",
    )
    .expect("standard NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(triage_key.clone(), ""))
    .with_parameter(
        "assignments",
        ParamValue::literal(json!([{ "name": "tier", "value": "standard" }])),
    );

    // Port-qualified edges drive routing: the engine fires only the matched port.
    let edge_true = Connection::new(triage_key.clone(), priority_key).with_from_port("true");
    let edge_false = Connection::new(triage_key, standard_key).with_from_port("false");

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-conditional-routing".into(),
        description: Some("core.if routes an order to priority or standard handling".into()),
        version: Version::new(0, 1, 0),
        nodes: vec![triage_node, priority_node, standard_node],
        connections: vec![edge_true, edge_false],
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
