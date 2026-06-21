//! End-to-end workflow example: a data pipeline built from first-party
//! `core.*` actions, executed through the real `WorkflowEngine`.
//!
//! This is the first runnable example that drives a *multi-node* workflow
//! through the engine. It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs`:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## The pipeline
//!
//! A list of order records flows through three connected nodes:
//!
//! ```text
//!   workflow input (orders)
//!        │
//!        ▼
//!   [filter]   core.filter   — keep only orders whose status == "shipped"
//!        │
//!        ▼
//!   [sort]     core.sort     — order the survivors by amount, descending
//!        │
//!        ▼
//!   [aggregate] core.aggregate — group by region: count orders + sum amounts
//! ```
//!
//! Each downstream node pulls its `data` from the upstream node's output via
//! `ParamValue::reference(<upstream node>, "")`, while its operation config
//! (`condition` / `keys` / `aggregations`) is supplied as a literal parameter.
//! The connections give the engine the execution order and make each
//! predecessor's output available to the next node.
//!
//! The example prints the input and the final aggregated output, then asserts
//! the exact result so it doubles as a smoke test: if the pipeline ever
//! regresses, the run fails loudly.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_data_pipeline
//! ```

use std::{collections::HashMap, sync::Arc};

use anyhow::Context as _;
use nebula_action::ActionResult;
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessRunner,
    WorkflowEngine,
};
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_plugin::ResolvedPlugin;
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, ParamValue, Version, WorkflowConfig,
    WorkflowDefinition,
};
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let orders = sample_orders();
    println!("=== Input: {} order records ===", order_count(&orders));
    println!("{}", pretty(&orders));

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_pipeline_workflow();

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            orders.clone(),
            ExecutionBudget::default(),
        )
        .await
        .context("executing the data-pipeline workflow")?;

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "pipeline must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );

    let aggregate_key = nebula_core::node_key!("aggregate");
    let summary = result
        .node_outputs
        .get(&aggregate_key)
        .context("aggregate node must have produced an output")?;

    println!("\n=== Output: per-region summary of shipped orders ===");
    println!("{}", pretty(summary));

    // Asserting the exact result turns this example into a smoke test: the
    // filter drops the cancelled/pending orders, the sort orders the shipped
    // survivors by amount (descending), and the aggregate groups them by region
    // in first-seen order — west (seen first at amount 100), then east.
    let expected = json!([
        { "region": "west", "order_count": 2, "total_amount": 130 },
        { "region": "east", "order_count": 2, "total_amount": 130 },
    ]);
    anyhow::ensure!(
        *summary == expected,
        "pipeline output must match the expected summary;\n expected: {}\n got:      {}",
        pretty(&expected),
        pretty(summary),
    );

    println!("\nPipeline output matches the expected summary.");
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
/// This mirrors `plugin_wiring_e2e.rs`'s `make_engine` + `with_plugin`: the
/// `ActionExecutor` here is the identity executor used by the engine's
/// in-process runner; the `core.*` actions themselves are registered by
/// `with_plugin(CorePlugin)`.
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

/// The concrete input: six order records across two regions and three statuses.
fn sample_orders() -> Value {
    json!([
        { "region": "west", "status": "shipped",   "amount": 100 },
        { "region": "east", "status": "shipped",   "amount": 50  },
        { "region": "west", "status": "cancelled", "amount": 999 },
        { "region": "west", "status": "shipped",   "amount": 30  },
        { "region": "east", "status": "pending",   "amount": 200 },
        { "region": "east", "status": "shipped",   "amount": 80  },
    ])
}

/// Build the three-node `filter -> sort -> aggregate` pipeline.
///
/// - `filter` reads the workflow-level input via `$input` (it is the entry
///   node, so its flow input is the orders array) and keeps `status == "shipped"`.
/// - `sort` references the filter output and orders it by `amount` descending.
/// - `aggregate` references the sort output and groups by `region`, counting
///   orders and summing amounts.
fn build_pipeline_workflow() -> WorkflowDefinition {
    let filter_key = nebula_core::node_key!("filter");
    let sort_key = nebula_core::node_key!("sort");
    let aggregate_key = nebula_core::node_key!("aggregate");

    // Entry node: pull the orders array from the workflow input ($input), then
    // keep only the shipped ones.
    let filter_node = NodeDefinition::new(
        filter_key.clone(),
        "Keep shipped orders",
        "core",
        "core.filter",
    )
    .expect("filter NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::expression("$input"))
    .with_parameter(
        "condition",
        ParamValue::literal(json!({ "field": "status", "op": "eq", "value": "shipped" })),
    );

    // Sort the shipped orders by amount, highest first.
    let sort_node = NodeDefinition::new(
        sort_key.clone(),
        "Sort by amount (desc)",
        "core",
        "core.sort",
    )
    .expect("sort NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(filter_key.clone(), ""))
    .with_parameter(
        "keys",
        ParamValue::literal(json!([{ "field": "amount", "order": "desc" }])),
    );

    // Group the sorted orders by region: count orders and sum amounts.
    let aggregate_node = NodeDefinition::new(
        aggregate_key.clone(),
        "Summarize per region",
        "core",
        "core.aggregate",
    )
    .expect("aggregate NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(sort_key.clone(), ""))
    .with_parameter("group_by", ParamValue::literal(json!(["region"])))
    .with_parameter(
        "aggregations",
        ParamValue::literal(json!([
            { "fn": "count", "out": "order_count" },
            { "fn": "sum", "field": "amount", "out": "total_amount" },
        ])),
    );

    // Connections drive execution order and publish each predecessor's output.
    let filter_to_sort = Connection::new(filter_key, sort_key.clone());
    let sort_to_aggregate = Connection::new(sort_key, aggregate_key);

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-data-pipeline".into(),
        description: Some("filter -> sort -> aggregate over order records".into()),
        version: Version::new(0, 1, 0),
        nodes: vec![filter_node, sort_node, aggregate_node],
        connections: vec![filter_to_sort, sort_to_aggregate],
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

/// Number of elements in a JSON array value, or 0 for any non-array.
fn order_count(orders: &Value) -> usize {
    orders.as_array().map_or(0, Vec::len)
}

/// Pretty-print a JSON value, falling back to its compact `Display` form if
/// pretty serialization somehow fails (it cannot for an in-memory `Value`).
fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
