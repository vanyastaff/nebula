//! End-to-end workflow example: **object reshaping** with `core.json_transform`,
//! executed through the real `WorkflowEngine`.
//!
//! The other workflow examples are row-oriented (`data_pipeline`), control-flow
//! (`conditional_routing`, `switch_router`), durable-wait (`delay_resume`), or
//! time arithmetic (`datetime_schedule`). This one reshapes the *structure* of a
//! single object — the everyday "normalize an external API payload" task: flatten
//! nested objects, strip secrets/internal fields, rename keys, and keep only a
//! public subset.
//!
//! It mirrors the standalone engine-run setup proven in
//! `crates/plugin-core/tests/plugin_wiring_e2e.rs` and reused by the sibling
//! workflow examples:
//!
//!   `ActionRegistry` -> `ActionExecutor` -> `InProcessRunner`
//!   -> `ActionRuntime` -> `WorkflowEngine::with_plugin(CorePlugin)`
//!
//! ## The workflow
//!
//! ```text
//!   workflow input: a nested API record { user{…}, account{…}, _internal{…} }
//!        │
//!        ▼
//!   [normalize] core.json_transform — flatten → omit secrets → rename → pick
//!        │  { id, name, tier, region }
//!        ▼
//!   [finalize]  core.set_fields     — stamp normalized = true
//! ```
//!
//! `core.json_transform` applies its `operations` left-to-right to the running
//! object; the reshaped record then flows to a `core.set_fields` node that
//! stamps a marker, proving the normalized object is usable downstream. Both the
//! intermediate reshape and the final output are asserted, so a reshape
//! regression (a leaked secret, a missed rename, a dropped field) fails loudly.
//!
//! ## Run it
//!
//! ```sh
//! cargo run -p nebula-examples --example workflow_json_reshape
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

    // A messy nested record from an upstream API: nested objects, a secret, and
    // an internal-only field we must not pass along.
    let raw_record = json!({
        "user": { "id": 7, "name": "Ada Lovelace", "password": "hunter2" },
        "account": { "tier": "pro", "region": "eu" },
        "_internal": { "trace": "abc-123" }
    });

    println!("=== Input: raw nested API record ===");
    println!("{}", pretty(&raw_record));

    let engine = build_engine().context("building the workflow engine")?;
    let workflow = build_reshape_workflow();

    let result = engine
        .execute_workflow(
            &nebula_engine::store_seam::single_tenant_scope(),
            &workflow,
            raw_record,
            ExecutionBudget::default(),
        )
        .await
        .context("executing the json-reshape workflow")?;

    anyhow::ensure!(
        result.status == ExecutionStatus::Completed,
        "workflow must reach Completed; got {:?} (node_errors: {:?})",
        result.status,
        result.node_errors,
    );

    let normalize_key = nebula_core::node_key!("normalize");
    let finalize_key = nebula_core::node_key!("finalize");

    // The reshape node produced the clean, flat public shape.
    let normalized = result
        .node_outputs
        .get(&normalize_key)
        .context("the `normalize` node must have produced an output")?;
    let expected_normalized = json!({
        "id": 7,
        "name": "Ada Lovelace",
        "tier": "pro",
        "region": "eu"
    });
    anyhow::ensure!(
        *normalized == expected_normalized,
        "reshape must flatten, strip secrets/internal, rename, and pick;\n expected: {}\n got:      {}",
        pretty(&expected_normalized),
        pretty(normalized),
    );
    // Explicit: the secret and internal field must NOT survive.
    anyhow::ensure!(
        normalized.get("password").is_none() && normalized.get("user.password").is_none(),
        "the password must never appear in the normalized record; got {}",
        pretty(normalized),
    );

    println!("\n=== After `normalize` (core.json_transform) ===");
    println!("{}", pretty(normalized));

    // The downstream node stamped its marker onto the reshaped record.
    let finalized = result
        .node_outputs
        .get(&finalize_key)
        .context("the `finalize` node must have produced an output")?;
    anyhow::ensure!(
        finalized["normalized"] == json!(true) && finalized["id"] == json!(7),
        "finalize must stamp normalized=true and preserve the reshaped fields; got {}",
        pretty(finalized),
    );

    println!("\n=== Final output (after `finalize` stamps the marker) ===");
    println!("{}", pretty(finalized));
    println!(
        "\nReshape verified: nested record normalized to a clean public shape, secret stripped."
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

/// Build the `normalize -> finalize` reshape workflow.
///
/// - `normalize` is the entry `core.json_transform` node: it reads the raw
///   record from the workflow input (`$input`) and reshapes it with an ordered
///   `operations` list — `flatten` (nested → dotted keys), `omit` (drop the
///   secret and the internal field), `rename` (dotted → clean keys), `pick`
///   (keep only the public subset).
/// - `finalize` is a `core.set_fields` node that stamps `normalized = true` onto
///   the reshaped record (referenced from `normalize`'s output).
fn build_reshape_workflow() -> WorkflowDefinition {
    let normalize_key = nebula_core::node_key!("normalize");
    let finalize_key = nebula_core::node_key!("finalize");

    // Operations apply left-to-right. `flatten` first so the top-level `omit`,
    // `rename`, and `pick` can address the now-dotted keys.
    let normalize_node = NodeDefinition::new(
        normalize_key.clone(),
        "Normalize the record",
        "core",
        "core.json_transform",
    )
    .expect("normalize NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::expression("$input"))
    .with_parameter(
        "operations",
        ParamValue::literal(json!([
            { "op": "flatten", "separator": "." },
            { "op": "omit", "fields": ["user.password", "_internal.trace"] },
            { "op": "rename", "from": "user.id", "to": "id" },
            { "op": "rename", "from": "user.name", "to": "name" },
            { "op": "rename", "from": "account.tier", "to": "tier" },
            { "op": "rename", "from": "account.region", "to": "region" },
            { "op": "pick", "fields": ["id", "name", "tier", "region"] },
        ])),
    );

    let finalize_node = NodeDefinition::new(
        finalize_key.clone(),
        "Stamp normalized marker",
        "core",
        "core.set_fields",
    )
    .expect("finalize NodeDefinition has valid keys")
    .with_parameter("data", ParamValue::reference(normalize_key.clone(), ""))
    .with_parameter(
        "assignments",
        ParamValue::literal(json!([{ "name": "normalized", "value": true }])),
    );

    let edge = Connection::new(normalize_key, finalize_key);

    let now = chrono::Utc::now();
    WorkflowDefinition {
        id: nebula_core::WorkflowId::new(),
        name: "workflow-json-reshape".into(),
        description: Some(
            "core.json_transform normalizes a nested record, then a marker is stamped".into(),
        ),
        version: Version::new(0, 1, 0),
        nodes: vec![normalize_node, finalize_node],
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
