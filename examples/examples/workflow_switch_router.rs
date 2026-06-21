//! End-to-end workflow example: **multi-way routing** with `core.switch`,
//! executed through the real `WorkflowEngine`.
//!
//! `workflow_conditional_routing` shows binary branching (`core.if`, two ports).
//! This is its multi-way complement: a `core.switch` node evaluates an ordered
//! list of `cases` and routes to the first matching port, falling through to a
//! `"default"` port when none match — the ubiquitous "router" node. The same
//! workflow is run three times (one per route) to exercise every port,
//! including the default, deterministically.
//!
//! It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs` (the `core.switch` e2e) and
//! reused by the sibling workflow examples:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## The workflow (support-ticket router)
//!
//! ```text
//!   workflow input: a ticket { id, severity, summary }
//!        │
//!        ▼
//!   [route]  core.switch   cases (in order):
//!        │     severity == "critical" → port "pager"
//!        │     severity == "high"     → port "oncall"
//!        │
//!        ├──"pager"───▶ [pager]   core.set_fields  → action="page-oncall-engineer"
//!        ├──"oncall"──▶ [oncall]  core.set_fields  → action="notify-oncall"
//!        └──"default"─▶ [queue]   core.set_fields  → action="backlog"
//! ```
//!
//! `core.switch` passes its `data` (the ticket) through on the selected port; the
//! branch `core.set_fields` node merges its `action` stamp onto that ticket. The
//! two ports that were not selected are **Skipped** — absent from both
//! `node_outputs` and `node_errors`.
//!
//! Each run asserts the selected branch ran with the right stamp *and* that the
//! other two were skipped, so a routing regression (wrong port, missed default,
//! or multiple branches firing) fails loudly.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_switch_router
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_router_workflow();

    let pager_key = nebula_core::node_key!("pager");
    let oncall_key = nebula_core::node_key!("oncall");
    let queue_key = nebula_core::node_key!("queue");

    println!("=== core.switch ticket router (route by severity) ===\n");

    // Each scenario picks a different port — including the `default` fall-through.
    let scenarios = [
        (
            json!({ "id": "T-1", "severity": "critical", "summary": "DB down" }),
            &pager_key,
            "page-oncall-engineer",
        ),
        (
            json!({ "id": "T-2", "severity": "high", "summary": "latency spike" }),
            &oncall_key,
            "notify-oncall",
        ),
        (
            json!({ "id": "T-3", "severity": "low", "summary": "typo in docs" }),
            &queue_key,
            "backlog",
        ),
    ];
    let all_branches = [
        (&pager_key, "pager"),
        (&oncall_key, "oncall"),
        (&queue_key, "queue"),
    ];

    for (ticket, expected_key, expected_action) in &scenarios {
        let ticket_id = ticket["id"].clone();
        let severity = ticket["severity"].as_str().unwrap_or("?");
        let result = run_once(&engine, &workflow, ticket).await?;

        // The selected branch ran, stamped its action, and preserved the ticket.
        let selected = result
            .node_outputs
            .get(*expected_key)
            .with_context(|| format!("ticket {ticket_id}: the selected branch must have run"))?;
        anyhow::ensure!(
            selected["action"] == json!(expected_action),
            "ticket {ticket_id}: selected branch must stamp action={expected_action}; got {selected}",
        );
        anyhow::ensure!(
            selected["id"] == ticket_id && selected["summary"] == ticket["summary"],
            "ticket {ticket_id}: selected branch must preserve the routed ticket; got {selected}",
        );

        // Every other branch was Skipped (no output AND no error).
        for (branch_key, branch_label) in &all_branches {
            if branch_key == expected_key {
                continue;
            }
            ensure_skipped(&result, branch_key, branch_label, &ticket_id)?;
        }

        println!(
            "severity {severity:>10} → {:<8} (action={expected_action}); other ports skipped",
            port_label(&all_branches, expected_key),
        );
    }

    println!("\nAll three routes verified, including the `default` fall-through.");
    Ok(())
}

/// Look up the human-readable port label for a branch key (for printing only).
fn port_label<'a>(
    branches: &[(&nebula_core::NodeKey, &'a str)],
    key: &nebula_core::NodeKey,
) -> &'a str {
    branches
        .iter()
        .find_map(|(k, label)| (*k == key).then_some(*label))
        .unwrap_or("?")
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
        .context("executing the switch-router workflow")?;

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
    ticket_id: &Value,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !result.node_outputs.contains_key(key),
        "ticket {ticket_id}: `{label}` must be Skipped (no output), but it produced one",
    );
    anyhow::ensure!(
        !result.node_errors.contains_key(key),
        "ticket {ticket_id}: `{label}` must be Skipped (no error), but it recorded one",
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

/// Build the `route -> {pager | oncall | queue}` router workflow.
///
/// - `route` is the entry `core.switch` node: it reads the ticket from the
///   workflow input (`$input`) and routes by `severity` to the first matching
///   case port, or the `"default"` port if none match.
/// - Each branch (`core.set_fields`) merges its `action` stamp onto the routed
///   ticket (the `data` `core.switch` emits on the selected port).
fn build_router_workflow() -> WorkflowDefinition {
    let route_key = nebula_core::node_key!("route");
    let pager_key = nebula_core::node_key!("pager");
    let oncall_key = nebula_core::node_key!("oncall");
    let queue_key = nebula_core::node_key!("queue");

    // Entry node: route on ticket severity. `data` is the workflow input ticket.
    // First matching case wins; `severity == "low"` (and anything else) falls
    // through to the auto-generated `"default"` port.
    let route_node = NodeDefinition::new(route_key.clone(), "Route by severity", "core", "core.switch")
        .expect("route NodeDefinition has valid keys")
        .with_parameter("data", ParamValue::expression("$input"))
        .with_parameter(
            "cases",
            ParamValue::literal(json!([
                { "condition": { "field": "severity", "op": "eq", "value": "critical" }, "port": "pager" },
                { "condition": { "field": "severity", "op": "eq", "value": "high" }, "port": "oncall" },
            ])),
        );

    let pager_node = branch_node(
        &pager_key,
        &route_key,
        "Page on-call",
        "page-oncall-engineer",
    );
    let oncall_node = branch_node(&oncall_key, &route_key, "Notify on-call", "notify-oncall");
    let queue_node = branch_node(&queue_key, &route_key, "Backlog queue", "backlog");

    // Port-qualified edges: case ports "pager"/"oncall" plus the "default" port.
    let edge_pager = Connection::new(route_key.clone(), pager_key).with_from_port("pager");
    let edge_oncall = Connection::new(route_key.clone(), oncall_key).with_from_port("oncall");
    let edge_queue = Connection::new(route_key, queue_key).with_from_port("default");

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-switch-router".into(),
        description: Some(
            "core.switch routes a ticket by severity to one of three handlers".into(),
        ),
        version: Version::new(0, 1, 0),
        nodes: vec![route_node, pager_node, oncall_node, queue_node],
        connections: vec![edge_pager, edge_oncall, edge_queue],
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

/// Build a branch handler: a `core.set_fields` node whose `data` is the ticket
/// routed by `route_key`, merging a single `action` stamp onto it.
fn branch_node(
    key: &nebula_core::NodeKey,
    route_key: &nebula_core::NodeKey,
    display_name: &str,
    action: &str,
) -> NodeDefinition {
    NodeDefinition::new(key.clone(), display_name, "core", "core.set_fields")
        .expect("branch NodeDefinition has valid keys")
        .with_parameter("data", ParamValue::reference(route_key.clone(), ""))
        .with_parameter(
            "assignments",
            ParamValue::literal(json!([{ "name": "action", "value": action }])),
        )
}
