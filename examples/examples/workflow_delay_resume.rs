//! Durable wait-state example: a `core.delay` node **parks** the execution on a
//! timer, the engine resumes it when the timer fires, and a downstream node runs
//! only after the resume — proving the park → timer → resume → complete cycle
//! end to end.
//!
//! This is the flagship differentiator the data-pipeline example does not show:
//! a node can suspend a running execution against an external condition (here, a
//! timer) and the engine durably resumes it. It mirrors the canonical drive in
//! `crates/plugin-core/tests/delay_e2e.rs`: spawn `execute_workflow` on a task,
//! subscribe to the engine's [`ExecutionEvent`] stream, observe `NodeParked`
//! (with a concrete `wake_at`) and `NodeWaitCompleted`, then await the spawned
//! task under a `tokio::time::timeout` backstop.
//!
//! ## The workflow
//!
//! ```text
//!   workflow input (a small JSON payload)
//!        │
//!        ▼
//!   [delay]   core.delay  — park 250ms on a timer (mode=for), pass `data` through
//!        │
//!        ▼  (gated until the timer fires)
//!   [resumed] core.set_fields — stamp `resumed_after_delay = true`
//! ```
//!
//! The `delay` node carries a `data` payload that is echoed downstream on resume;
//! the `resumed` node stamps a marker field so the final output proves the
//! pipeline actually resumed after the park.
//!
//! ## A real timer is used deliberately
//!
//! The wait-state scheduler runs on its own task inside `execute_workflow`, so a
//! `start_paused` virtual clock cannot drive it (the example cannot advance the
//! scheduler's time). The delay is a real, sub-second `for` span of 250 ms —
//! `core.delay`'s finest unit is `DurationUnit::Milliseconds` — bounded by a
//! generous `tokio::time::timeout` backstop, so a regression fails fast instead
//! of hanging. The printed elapsed wall-clock time shows the real park happened.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_delay_resume
//! ```
//!
//! It completes in a fraction of a second (the real timer span) plus engine overhead.

use std::{collections::HashMap, sync::Arc, time::Duration, time::Instant};

use anyhow::Context as _;
use nebula_action::ActionResult;
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, DataPassingPolicy, ExecutionEvent,
    InProcessRunner, ResolvedPlugin, WorkflowEngine,
};
use nebula_eventbus::EventBus;
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_metrics::MetricsRegistry;
use nebula_plugin_core::CorePlugin;
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, ParamValue, Version, WorkflowConfig,
    WorkflowDefinition,
};
use serde_json::{Value, json};

/// How long the delay node parks, in milliseconds. 250 ms is comfortably
/// sub-second — `core.delay`'s finest unit is `DurationUnit::Milliseconds` — yet
/// long enough that the park is observably real (not instantaneous).
const DELAY_MILLIS: u64 = 250;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let payload = json!({ "order_id": "A-1001", "note": "carried through the park" });
    println!("=== Input payload (passed through the delay node) ===");
    println!("{}", pretty(&payload));
    println!("\nBuilding a `delay → set_fields` workflow with a {DELAY_MILLIS}ms timer park.\n");

    let delay_key = nebula_core::node_key!("delay");
    let resumed_key = nebula_core::node_key!("resumed");
    let workflow = Arc::new(build_delay_workflow(
        delay_key.clone(),
        resumed_key.clone(),
        &payload,
    ));

    // Wire the engine with the CorePlugin and an event bus so we can narrate the
    // park/resume as the engine emits it.
    let event_bus = EventBus::<ExecutionEvent>::new(128);
    let mut events = event_bus.subscribe();
    let engine = Arc::new(
        build_engine()
            .context("building the workflow engine")?
            .with_event_bus(event_bus),
    );

    // Run the workflow on its own task: it will park on the timer, so we cannot
    // simply `.await` it before observing the park.
    let started_at = Instant::now();
    let engine_for_task = Arc::clone(&engine);
    let workflow_for_task = Arc::clone(&workflow);
    let run = tokio::spawn(async move {
        engine_for_task
            .execute_workflow(
                &nebula_engine::store_seam::single_tenant_scope(),
                &workflow_for_task,
                json!(null),
                ExecutionBudget::default(),
            )
            .await
    });

    // Narrate the park: wait for the delay node to park and confirm the timer
    // carries a concrete `wake_at` (the timer-variant proof). Bounded so a
    // regression that never parks fails fast.
    let wake_at = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(ExecutionEvent::NodeParked {
                    node_key, wake_at, ..
                }) if node_key == delay_key => break Ok(wake_at),
                Some(_) => continue,
                None => {
                    break Err(anyhow::anyhow!(
                        "event stream closed before the node parked"
                    ));
                },
            }
        }
    })
    .await
    .context("timed out waiting for the delay node to park (the timer-wait regressed?)")??;

    let wake_at = wake_at.context("a timer (for) delay must carry a concrete wake_at")?;
    println!(
        "⏸  node `delay` parked — waiting {DELAY_MILLIS}ms for the timer (wake_at = {})",
        wake_at.to_rfc3339()
    );

    // Narrate the resume: the matched-pair `NodeWaitCompleted` fires when the
    // timer fires and the node leaves `Waiting`.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Some(ExecutionEvent::NodeWaitCompleted { node_key, .. })
                    if node_key == delay_key =>
                {
                    break Ok(());
                },
                Some(_) => continue,
                None => {
                    break Err(anyhow::anyhow!(
                        "event stream closed before the timer fired"
                    ));
                },
            }
        }
    })
    .await
    .context("timed out waiting for the delay timer to fire (the resume path regressed?)")??;

    println!("▶  timer fired — execution resumed; downstream node now released");

    // Await the spawned run under a backstop so a stuck resume fails fast.
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .context("the workflow did not finish within 10s after resume")?
        .context("the workflow task panicked")?
        .context("executing the delay workflow")?;

    let elapsed = started_at.elapsed();

    // ── Assertions: the example doubles as a CI-rot-guarded smoke test ──────────

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "execution must reach Completed after the delay fires; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );

    // The delay node passed its input `data` through to its output on resume.
    let delay_output = result
        .node_outputs
        .get(&delay_key)
        .context("the delay node must have produced an output after resume")?;
    anyhow::ensure!(
        delay_output["order_id"] == json!("A-1001"),
        "the delay node must pass its input data through on resume; got {}",
        pretty(delay_output),
    );

    // The downstream node ran only after the resume and stamped its marker.
    let resumed_output = result
        .node_outputs
        .get(&resumed_key)
        .context("the downstream node must have produced an output after resume")?;
    anyhow::ensure!(
        resumed_output["resumed_after_delay"] == json!(true),
        "the downstream node must stamp `resumed_after_delay = true`; got {}",
        pretty(resumed_output),
    );

    // The park was real: wall-clock elapsed must cover the timer span.
    anyhow::ensure!(
        elapsed >= Duration::from_millis(DELAY_MILLIS),
        "wall-clock elapsed ({elapsed:?}) must be at least the {DELAY_MILLIS}ms timer span — \
         a near-instant run means the node did not actually park",
    );

    println!("\n=== Final output of the `resumed` node ===");
    println!("{}", pretty(resumed_output));
    let elapsed_secs = elapsed.as_secs_f64();
    println!(
        "\nCompleted in {elapsed_secs:.3}s of real wall-clock time \
         ({DELAY_MILLIS}ms timer park + engine overhead).",
    );
    println!("Park → timer → resume → complete: the durable wait-state worked end to end.");
    Ok(())
}

/// Initialise a simple `fmt` tracing subscriber so the engine's instrumentation
/// is visible when `RUST_LOG` is set (e.g. `RUST_LOG=info`).
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
/// Mirrors the data-pipeline example: the `ActionExecutor` is the identity
/// executor used by the in-process runner; the `core.*` actions themselves are
/// registered by `with_plugin(CorePlugin)`.
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

/// Build the two-node `delay → set_fields` workflow.
///
/// - `delay` is a `core.delay` node in `for` mode (250 ms). It carries the given
///   `payload` as its `data`, which the engine echoes downstream on resume.
/// - `resumed` is a `core.set_fields` node that stamps `resumed_after_delay` so
///   the output proves it ran after — and only after — the timer fired.
fn build_delay_workflow(
    delay_key: nebula_core::NodeKey,
    resumed_key: nebula_core::NodeKey,
    payload: &Value,
) -> WorkflowDefinition {
    // Wire shape verified against `DelaySpec::For { amount, unit }` /
    // `DurationUnit::Milliseconds`: {"mode":"for","amount":250,"unit":"milliseconds"}.
    let delay_node =
        NodeDefinition::new(delay_key.clone(), "Park on a timer", "core", "core.delay")
            .expect("delay NodeDefinition has valid keys")
            .with_parameter("mode", ParamValue::literal(json!("for")))
            .with_parameter("amount", ParamValue::literal(json!(DELAY_MILLIS)))
            .with_parameter("unit", ParamValue::literal(json!("milliseconds")))
            .with_parameter("data", ParamValue::literal(payload.clone()));

    let resumed_node = NodeDefinition::new(
        resumed_key.clone(),
        "Mark resumed",
        "core",
        "core.set_fields",
    )
    .expect("set_fields NodeDefinition has valid keys")
    .with_parameter(
        "assignments",
        ParamValue::literal(json!([{ "name": "resumed_after_delay", "value": true }])),
    );

    let edge = Connection::new(delay_key, resumed_key);

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-delay-resume".into(),
        description: Some("core.delay parks on a timer, then resumes downstream".into()),
        version: Version::new(0, 1, 0),
        nodes: vec![delay_node, resumed_node],
        connections: vec![edge],
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

/// Pretty-print a JSON value, falling back to its compact `Display` form if
/// pretty serialization somehow fails (it cannot for an in-memory `Value`).
fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
