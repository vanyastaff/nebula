//! End-to-end workflow example: offset-aware, **millisecond-precise** timestamp
//! arithmetic built from the first-party `core.datetime` action, executed
//! through the real `WorkflowEngine`.
//!
//! It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs` and reused by
//! `workflow_data_pipeline`:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## Why this example exists
//!
//! `core.datetime` is millisecond-precise across every op (`parse` preserves
//! sub-second input, `add`/`subtract` preserve sub-second results, `diff`
//! measures in milliseconds, `format` renders whatever the strftime string
//! asks for). Sub-second precision is the difference between expressing a real
//! polling/back-off interval and rounding it away — so this example computes a
//! **next-poll instant** from an event time, advancing it by a 1 500 ms
//! interval that crosses a whole-second boundary.
//!
//! ## The pipeline
//!
//! ```text
//!   workflow input: "2026-06-21T09:00:00.250+02:00"   (an event time, +02:00, .250s)
//!        │
//!        ▼
//!   [normalize]  core.datetime parse   — offset → canonical UTC, sub-second kept
//!        │  "2026-06-21T07:00:00.250Z"
//!        ▼
//!   [next_poll]  core.datetime add     — + 1 500 ms back-off interval
//!        │  "2026-06-21T07:00:01.750Z"   (.250 + 1.500s crosses a whole second)
//!        ▼
//!   [render]     core.datetime format  — strftime with %.3f sub-second digits
//!           "2026-06-21 07:00:01.750 UTC"
//! ```
//!
//! Each downstream node pulls its `input` from the upstream node's output via
//! `ParamValue::reference(<upstream node>, "")`; the op config (`op` / `amount`
//! / `unit` / `format`) is supplied as literal parameters. The connections give
//! the engine the execution order and publish each predecessor's output.
//!
//! The example prints each step and asserts the exact result, so it doubles as a
//! smoke test: an offset bug, a sub-second-truncation regression, or a
//! mis-computed interval each cause a distinct, loud failure.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_datetime_schedule
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

/// The event time observed from an external source: a non-UTC offset (+02:00)
/// with a sub-second component (.250). Both must survive into the schedule.
const EVENT_TIME: &str = "2026-06-21T09:00:00.250+02:00";

/// The poll back-off interval, in milliseconds. 1 500 ms is deliberately
/// sub-second-bearing and crosses a whole-second boundary when added to the
/// `.250` event time, so a truncation regression is visible in the output.
const POLL_INTERVAL_MILLIS: i64 = 1_500;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    println!("=== Input: event time (offset-aware, sub-second) ===");
    println!("{EVENT_TIME}");
    println!(
        "\nComputing the next poll instant: normalize to UTC, add a \
         {POLL_INTERVAL_MILLIS}ms interval, then format.\n"
    );

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_schedule_workflow();

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            json!(EVENT_TIME),
            ExecutionBudget::default(),
        )
        .await
        .context("executing the datetime-schedule workflow")?;

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "workflow must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );

    let normalize_key = nebula_core::node_key!("normalize");
    let next_poll_key = nebula_core::node_key!("next_poll");
    let render_key = nebula_core::node_key!("render");

    let normalized = node_output(&result, &normalize_key, "normalize")?;
    let next_poll = node_output(&result, &next_poll_key, "next_poll")?;
    let rendered = node_output(&result, &render_key, "render")?;

    println!("⏱  normalized event time (UTC)   : {normalized}");
    println!("⏭  next poll instant (+{POLL_INTERVAL_MILLIS}ms): {next_poll}");
    println!("🖉  rendered for a log line       : {rendered}");

    // Strong-witness assertions — each step must have done the right thing:
    //
    // normalize: +02:00 of 09:00:00.250 == 07:00:00.250 UTC. A dropped offset
    //   would give 09:00:00.250Z; a truncated sub-second would give 07:00:00Z.
    anyhow::ensure!(
        *normalized == json!("2026-06-21T07:00:00.250Z"),
        "normalize must shift the offset to UTC AND preserve .250; got {normalized}",
    );

    // add: 07:00:00.250 + 1.500s == 07:00:01.750. Crossing the whole second
    //   proves millisecond arithmetic (a seconds-only base would land on
    //   07:00:01.250 at best, or 07:00:01 if the interval were rounded down).
    anyhow::ensure!(
        *next_poll == json!("2026-06-21T07:00:01.750Z"),
        "next poll must be the event time + 1.500s, sub-second preserved; got {next_poll}",
    );

    // format: strftime with %.3f renders the sub-second digits explicitly.
    anyhow::ensure!(
        *rendered == json!("2026-06-21 07:00:01.750 UTC"),
        "render must format the next-poll instant with millisecond digits; got {rendered}",
    );

    println!("\nSchedule computed with millisecond precision, end to end.");
    Ok(())
}

/// Initialise a simple `fmt` tracing subscriber so the `core.datetime`
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
/// Mirrors `workflow_data_pipeline`'s `build_engine`: the `ActionExecutor` is
/// the identity executor used by the in-process runner; the `core.*` actions
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

/// Build the three-node `normalize -> next_poll -> render` schedule workflow.
///
/// - `normalize` is the entry node: it reads the event time from the workflow
///   input (`$input`) and `parse`s it to canonical UTC (offset shifted,
///   sub-second preserved).
/// - `next_poll` references the normalized instant and `add`s the back-off
///   interval in `milliseconds`.
/// - `render` references the next-poll instant and `format`s it with a strftime
///   string whose `%.3f` emits the sub-second digits.
fn build_schedule_workflow() -> WorkflowDefinition {
    let normalize_key = nebula_core::node_key!("normalize");
    let next_poll_key = nebula_core::node_key!("next_poll");
    let render_key = nebula_core::node_key!("render");

    // Entry node: normalize the workflow-input timestamp to canonical UTC.
    // Wire shape: {"op":"parse","input":"<$input>"}.
    let normalize_node = NodeDefinition::new(
        normalize_key.clone(),
        "Normalize event time to UTC",
        "core",
        "core.datetime",
    )
    .expect("normalize NodeDefinition has valid keys")
    .with_parameter("op", ParamValue::literal(json!("parse")))
    .with_parameter("input", ParamValue::expression("$input"));

    // Advance by the back-off interval, in milliseconds.
    // Wire shape: {"op":"add","input":"<normalize>","amount":1500,"unit":"milliseconds"}.
    let next_poll_node = NodeDefinition::new(
        next_poll_key.clone(),
        "Add the poll interval",
        "core",
        "core.datetime",
    )
    .expect("next_poll NodeDefinition has valid keys")
    .with_parameter("op", ParamValue::literal(json!("add")))
    .with_parameter("input", ParamValue::reference(normalize_key.clone(), ""))
    .with_parameter("amount", ParamValue::literal(json!(POLL_INTERVAL_MILLIS)))
    .with_parameter("unit", ParamValue::literal(json!("milliseconds")));

    // Format the next-poll instant for a human-readable log line.
    // Wire shape: {"op":"format","input":"<next_poll>","format":"..."}.
    let render_node = NodeDefinition::new(
        render_key.clone(),
        "Format for a log line",
        "core",
        "core.datetime",
    )
    .expect("render NodeDefinition has valid keys")
    .with_parameter("op", ParamValue::literal(json!("format")))
    .with_parameter("input", ParamValue::reference(next_poll_key.clone(), ""))
    .with_parameter(
        "format",
        ParamValue::literal(json!("%Y-%m-%d %H:%M:%S%.3f UTC")),
    );

    // Connections drive execution order and publish each predecessor's output.
    let normalize_to_next = Connection::new(normalize_key, next_poll_key.clone());
    let next_to_render = Connection::new(next_poll_key, render_key);

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-datetime-schedule".into(),
        description: Some("parse -> add (ms interval) -> format with core.datetime".into()),
        version: Version::new(0, 1, 0),
        nodes: vec![normalize_node, next_poll_node, render_node],
        connections: vec![normalize_to_next, next_to_render],
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

/// Fetch a node's primary output value, or fail with a clear context message.
fn node_output<'a>(
    result: &'a nebula_engine::ExecutionResult,
    key: &nebula_core::NodeKey,
    label: &str,
) -> anyhow::Result<&'a Value> {
    result
        .node_outputs
        .get(key)
        .with_context(|| format!("the `{label}` node must have produced an output"))
}
