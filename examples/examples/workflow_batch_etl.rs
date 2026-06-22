//! End-to-end workflow example: a **batch ETL pipeline** that composes three
//! first-party array actions — `core.dedupe`, `core.map`, and `core.array` —
//! through the real `WorkflowEngine`.
//!
//! `data_pipeline` shows filter/sort/aggregate; this is the other half of the
//! array toolbox: deduplicate records by key, project each one to a clean shape,
//! then group them into fixed-size batches ready for a downstream sink.
//!
//! It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs` and reused by the sibling
//! workflow examples:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## The pipeline
//!
//! ```text
//!   workflow input: raw events (with a duplicate id + a secret field)
//!        │
//!        ▼
//!   [dedupe]  core.dedupe  — keep the first event per `id` (order preserved)
//!        │
//!        ▼
//!   [project] core.map     — per element: omit `secret`, rename first_name → name
//!        │
//!        ▼
//!   [batch]   core.array   — chunk the projection into batches of 2
//! ```
//!
//! Each downstream node pulls its `data` from the upstream node's output via
//! `ParamValue::reference(<upstream node>, "")`; the op config is a literal
//! parameter. Every stage is asserted (dedup drops the duplicate, the secret
//! never survives projection, the final batching has the right shape), so the
//! example doubles as a smoke test.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_batch_etl
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

    // Raw events: id 1 appears twice (the second must be dropped), and every
    // record carries a `secret` that must not survive projection.
    let raw_events = json!([
        { "id": 1, "first_name": "Ada",     "secret": "tok-1" },
        { "id": 2, "first_name": "Bob",     "secret": "tok-2" },
        { "id": 1, "first_name": "Ada-dup", "secret": "tok-3" },
        { "id": 3, "first_name": "Cara",    "secret": "tok-4" }
    ]);

    println!(
        "=== Input: {} raw events (id 1 duplicated) ===",
        len(&raw_events)
    );
    println!("{}", pretty(&raw_events));

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_batch_workflow();

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            raw_events,
            ExecutionBudget::default(),
        )
        .await
        .context("executing the batch-ETL workflow")?;

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "pipeline must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );

    let dedupe_key = nebula_core::node_key!("dedupe");
    let project_key = nebula_core::node_key!("project");
    let batch_key = nebula_core::node_key!("batch");

    // dedupe: the second id=1 ("Ada-dup") is dropped; first-seen order preserved.
    let deduped = result
        .node_outputs
        .get(&dedupe_key)
        .context("the `dedupe` node must have produced an output")?;
    anyhow::ensure!(
        len(deduped) == 3,
        "dedupe must keep 3 unique-by-id events; got {}",
        pretty(deduped),
    );
    anyhow::ensure!(
        !contains_name(deduped, "Ada-dup"),
        "dedupe must drop the second id=1 event; got {}",
        pretty(deduped),
    );

    // project: secret stripped, first_name renamed to name, on every element.
    let projected = result
        .node_outputs
        .get(&project_key)
        .context("the `project` node must have produced an output")?;
    let projected_array = projected
        .as_array()
        .context("project output must be an array")?;
    anyhow::ensure!(
        projected_array
            .iter()
            .all(|element| element.get("secret").is_none() && element.get("name").is_some()),
        "every projected element must drop `secret` and have `name`; got {}",
        pretty(projected),
    );

    // batch: chunked into [2, 1].
    let batched = result
        .node_outputs
        .get(&batch_key)
        .context("the `batch` node must have produced an output")?;
    let expected_batches = json!([
        [ { "id": 1, "name": "Ada" }, { "id": 2, "name": "Bob" } ],
        [ { "id": 3, "name": "Cara" } ]
    ]);
    anyhow::ensure!(
        *batched == expected_batches,
        "batching must group the projection into [2, 1];\n expected: {}\n got:      {}",
        pretty(&expected_batches),
        pretty(batched),
    );

    println!("\n=== Output: deduplicated, projected, batched into groups of 2 ===");
    println!("{}", pretty(batched));
    println!(
        "\nBatch ETL verified: {} raw events → 3 unique → projected → {} batches.",
        4,
        len(batched),
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

/// Build the `dedupe -> project -> batch` ETL workflow.
///
/// - `dedupe` is the entry `core.dedupe` node: it reads the raw events from the
///   workflow input (`$input`) and keeps the first event per `id`.
/// - `project` is a `core.map` node: it reshapes each surviving element (omit the
///   secret, rename `first_name` → `name`).
/// - `batch` is a `core.array` node: it chunks the projection into groups of 2.
fn build_batch_workflow() -> WorkflowDefinition {
    let dedupe_key = nebula_core::node_key!("dedupe");
    let project_key = nebula_core::node_key!("project");
    let batch_key = nebula_core::node_key!("batch");

    // Entry: dedupe the raw events by id (first occurrence wins).
    let dedupe_node =
        NodeDefinition::new(dedupe_key.clone(), "Dedupe by id", "core", "core.dedupe")
            .expect("dedupe NodeDefinition has valid keys")
            .with_parameter("data", ParamValue::expression("$input"))
            .with_parameter("keys", ParamValue::literal(json!(["id"])));

    // Project each element to a clean public shape.
    let project_node = NodeDefinition::new(
        project_key.clone(),
        "Project each event",
        "core",
        "core.map",
    )
    .expect("project NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(dedupe_key.clone(), ""))
    .with_parameter(
        "operations",
        ParamValue::literal(json!([
            { "op": "omit", "fields": ["secret"] },
            { "op": "rename", "from": "first_name", "to": "name" },
        ])),
    );

    // Group the projection into fixed-size batches.
    let batch_node = NodeDefinition::new(
        batch_key.clone(),
        "Batch into groups of 2",
        "core",
        "core.array",
    )
    .expect("batch NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(project_key.clone(), ""))
    .with_parameter(
        "operations",
        ParamValue::literal(json!([{ "op": "chunk", "size": 2 }])),
    );

    let dedupe_to_project = Connection::new(dedupe_key, project_key.clone());
    let project_to_batch = Connection::new(project_key, batch_key);

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-batch-etl".into(),
        description: Some("core.dedupe -> core.map -> core.array batch ETL".into()),
        version: Version::new(0, 1, 0),
        nodes: vec![dedupe_node, project_node, batch_node],
        connections: vec![dedupe_to_project, project_to_batch],
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
fn len(value: &Value) -> usize {
    value.as_array().map_or(0, Vec::len)
}

/// Whether any object in `array` has a `first_name` equal to `name`.
fn contains_name(array: &Value, name: &str) -> bool {
    array.as_array().is_some_and(|elements| {
        elements
            .iter()
            .any(|element| element["first_name"] == json!(name))
    })
}

/// Pretty-print a JSON value, falling back to its compact `Display` form if
/// pretty serialization somehow fails (it cannot for an in-memory `Value`).
fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
