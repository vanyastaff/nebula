use std::{collections::HashMap, process::ExitCode, sync::Arc};

use anyhow::Context;
use nebula_core::{NodeKey, id::ExecutionId};
use nebula_engine::WorkflowEngine;
use nebula_execution::{ExecutionStatus, ReplayPlan, context::ExecutionBudget};
use nebula_runtime::{ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox};
use nebula_telemetry::metrics::MetricsRegistry;

use crate::cli::{OutputFormat, ReplayArgs, resolve_format};

/// Execute the `replay` command.
pub async fn execute(args: ReplayArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    // 1. Parse workflow.
    let content = std::fs::read_to_string(&args.workflow)
        .with_context(|| format!("failed to read {}", args.workflow.display()))?;
    let definition = super::validate::parse_workflow_lenient(&content, &args.workflow)?;

    // 2. Find the target node by name.
    let target_node = definition
        .nodes
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(&args.from))
        .ok_or_else(|| {
            let names: Vec<&str> = definition.nodes.iter().map(|n| n.name.as_str()).collect();
            anyhow::anyhow!(
                "node \"{}\" not found.\nAvailable: {}",
                args.from,
                names.join(", ")
            )
        })?;

    // 3. Load pinned outputs (from file or empty).
    let pinned_outputs: HashMap<NodeKey, serde_json::Value> = if let Some(ref outputs_file) =
        args.outputs_file
    {
        let content = std::fs::read_to_string(outputs_file)
            .with_context(|| format!("failed to read {}", outputs_file.display()))?;
        let raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(&content).context("invalid outputs JSON")?;
        raw.into_iter()
                .filter_map(|(k, v)| match k.parse::<NodeKey>() {
                    Ok(id) => Some((id, v)),
                    Err(e) => {
                        tracing::warn!(key = %k, error = %e, "skipping pinned output with unparsable NodeKey");
                        None
                    },
                })
                .collect()
    } else {
        HashMap::new()
    };

    // 4. Build replay plan.
    let input_override: serde_json::Value =
        serde_json::from_str(&args.input).context("failed to parse --input as JSON")?;

    let mut plan = ReplayPlan::new(ExecutionId::new(), target_node.id.clone());
    plan.pinned_outputs = pinned_outputs;
    if input_override != serde_json::Value::Object(serde_json::Map::new()) {
        plan.input_overrides
            .insert(target_node.id.clone(), input_override);
    }

    // 5. Build engine.
    let registry = Arc::new(ActionRegistry::new());

    let community_count = crate::plugins::discover_and_register(&registry).await;
    if community_count > 0 {
        tracing::info!(count = community_count, "registered community plugins");
    }

    let metrics = MetricsRegistry::new();
    // Sandbox executor is unreachable in Phase 7.5 — see run.rs for details.
    let executor: nebula_runtime::sandbox::ActionExecutor = Arc::new(|_ctx, _metadata, _input| {
        Box::pin(async move {
            Err(nebula_action::ActionError::fatal(
                "sandbox executor invoked unexpectedly — Phase 7.5 routes all execution \
                 through ActionHandler enum dispatch, sandbox is Phase 7.6 work",
            ))
        })
    });

    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let runtime = Arc::new(ActionRuntime::new(
        Arc::clone(&registry),
        sandbox,
        DataPassingPolicy::default(),
        metrics.clone(),
    ));

    let engine = WorkflowEngine::new(runtime, metrics);
    let budget = ExecutionBudget::default();

    // 6. Execute replay.
    if !quiet {
        eprintln!(
            "Replaying from node \"{}\" ({})",
            target_node.name,
            target_node.id.clone()
        );
    }

    let result = engine.replay_execution(&definition, plan, budget).await?;

    // 7. Output.
    if !quiet {
        let format = resolve_format(args.format);
        match format {
            OutputFormat::Json => {
                let outputs: serde_json::Map<String, serde_json::Value> = result
                    .node_outputs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect();
                let errors: serde_json::Map<String, serde_json::Value> = result
                    .node_errors
                    .iter()
                    .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.clone())))
                    .collect();
                let json = serde_json::json!({
                    "execution_id": result.execution_id.to_string(),
                    "status": format!("{:?}", result.status),
                    "duration_ms": result.duration.as_millis(),
                    "outputs": outputs,
                    "errors": errors,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
                );
            },
            OutputFormat::Text => {
                println!("Replay:    {}", result.execution_id);
                println!("Status:    {:?}", result.status);
                println!("Duration:  {:?}", result.duration);
                println!("Nodes:     {}", result.node_outputs.len());
                if !result.node_errors.is_empty() {
                    println!("\nNode errors:");
                    for (nid, err) in &result.node_errors {
                        println!("  {nid}: {err}");
                    }
                }
            },
        }
    }

    Ok(match result.status {
        ExecutionStatus::Completed => ExitCode::SUCCESS,
        _ => ExitCode::from(2),
    })
}
