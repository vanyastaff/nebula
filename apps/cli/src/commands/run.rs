use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use nebula_engine::WorkflowEngine;
use nebula_execution::ExecutionStatus;
use nebula_execution::context::ExecutionBudget;
use nebula_runtime::{ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox};
use nebula_telemetry::metrics::MetricsRegistry;

use crate::cli::{OutputFormat, RunArgs, resolve_format};

/// Execute the `run` command.
pub async fn execute(args: RunArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    // 1. Load and parse workflow.
    let content = std::fs::read_to_string(&args.workflow)
        .with_context(|| format!("failed to read {}", args.workflow.display()))?;

    let definition = super::validate::parse_workflow(&content, &args.workflow)?;

    // 2. Parse input data (from --input or --input-file).
    let input: serde_json::Value = load_input(&args)?;

    // 3. Build the execution stack.
    let registry = Arc::new(ActionRegistry::new());
    crate::actions::register_builtins(&registry);

    // Discover and register community plugins from plugins/ directories.
    let community_count = crate::plugins::discover_and_register(&registry).await;
    if community_count > 0 {
        tracing::info!(
            count = community_count,
            "registered community plugin actions"
        );
    }

    let metrics = MetricsRegistry::new();

    let registry_for_sandbox = Arc::clone(&registry);
    let executor: nebula_runtime::sandbox::ActionExecutor =
        Arc::new(move |ctx, metadata, input| {
            let registry = Arc::clone(&registry_for_sandbox);
            let key = metadata.key.as_str().to_owned();
            Box::pin(async move {
                let handler = registry
                    .get(&key)
                    .map_err(|e| nebula_action::ActionError::fatal(e.to_string()))?;
                handler.execute(input, ctx.inner()).await
            })
        });

    let sandbox = Arc::new(InProcessSandbox::new(executor));
    let data_policy = DataPassingPolicy::default();
    let runtime = Arc::new(ActionRuntime::new(
        Arc::clone(&registry),
        sandbox,
        data_policy,
        metrics.clone(),
    ));

    let engine = WorkflowEngine::new(runtime, metrics);

    // 4. Build execution budget.
    let budget = ExecutionBudget {
        max_concurrent_nodes: args.concurrency,
        max_duration: args.timeout,
        max_output_bytes: None,
        max_total_retries: None,
    };

    // 5. Execute.
    if args.stream && !quiet {
        eprintln!("Executing {}...", args.workflow.display());
    }

    let result = engine.execute_workflow(&definition, input, budget).await?;

    // 6. Output result.
    #[cfg(feature = "tui")]
    if args.tui {
        return run_tui_view(&definition, &result).await;
    }

    if !quiet {
        let format = resolve_format(args.format);
        match format {
            OutputFormat::Json => print_json_result(&result),
            OutputFormat::Text => print_text_result(&result),
        }
    }

    // 7. Exit code based on workflow status.
    Ok(exit_code_for_status(&result.status))
}

fn load_input(args: &RunArgs) -> anyhow::Result<serde_json::Value> {
    match &args.input_file {
        Some(path) => {
            let content = if path.to_str() == Some("-") {
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .context("failed to read stdin")?;
                buf
            } else {
                std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read {}", path.display()))?
            };
            serde_json::from_str(&content).context("failed to parse input file as JSON")
        }
        None => serde_json::from_str(&args.input).context("failed to parse --input as JSON"),
    }
}

fn print_json_result(result: &nebula_engine::ExecutionResult) {
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
        serde_json::to_string_pretty(&json).expect("json serialization")
    );
}

fn print_text_result(result: &nebula_engine::ExecutionResult) {
    println!("Execution: {}", result.execution_id);
    println!("Status:    {:?}", result.status);
    println!("Duration:  {:?}", result.duration);
    println!("Nodes:     {}", result.node_outputs.len());

    if !result.node_outputs.is_empty() {
        println!("\nNode outputs:");
        for (node_id, output) in &result.node_outputs {
            let json = serde_json::to_string(output).unwrap_or_else(|_| "???".to_owned());
            let truncated = if json.len() > 200 {
                format!("{}...", &json[..200])
            } else {
                json
            };
            println!("  {node_id}: {truncated}");
        }
    }

    if !result.node_errors.is_empty() {
        println!("\nNode errors:");
        for (node_id, error) in &result.node_errors {
            println!("  {node_id}: {error}");
        }
    }
}

fn exit_code_for_status(status: &ExecutionStatus) -> ExitCode {
    match status {
        ExecutionStatus::Completed => ExitCode::SUCCESS,
        ExecutionStatus::TimedOut => ExitCode::from(super::exit_codes::TIMEOUT),
        _ => ExitCode::from(super::exit_codes::WORKFLOW_FAILED),
    }
}

/// Launch the TUI view with results from a completed execution.
#[cfg(feature = "tui")]
async fn run_tui_view(
    workflow: &nebula_workflow::WorkflowDefinition,
    result: &nebula_engine::ExecutionResult,
) -> anyhow::Result<ExitCode> {
    use crate::tui::app::{App, NodeStatus};
    use crate::tui::event::{LogLevel, TuiEvent};

    // Build node order from workflow definition.
    let node_order: Vec<_> = workflow
        .nodes
        .iter()
        .map(|n| (n.id, n.name.clone(), n.action_key.to_string()))
        .collect();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        workflow.name.clone(),
        result.execution_id.to_string(),
        node_order,
    );

    // Populate app state from the completed result.
    for (node_id, output) in &result.node_outputs {
        if let Some(&idx) = app.node_index.get(node_id) {
            app.nodes[idx].1.status = NodeStatus::Completed;
            app.nodes[idx].1.output = Some(output.clone());
        }
    }
    for (node_id, error) in &result.node_errors {
        if let Some(&idx) = app.node_index.get(node_id) {
            app.nodes[idx].1.status = NodeStatus::Failed;
            app.nodes[idx].1.error = Some(error.clone());
        }
    }

    // Set durations from the overall result.
    for (_, info) in &mut app.nodes {
        if info.status == NodeStatus::Completed || info.status == NodeStatus::Failed {
            // Individual node durations not available yet — show total.
            info.elapsed = Some(result.duration);
        }
    }

    app.done = true;
    app.success = result.status == ExecutionStatus::Completed;

    // Send completion event for the log panel.
    let _ = tx.send(TuiEvent::Log {
        level: if app.success {
            LogLevel::Info
        } else {
            LogLevel::Error
        },
        message: format!("execution {:?} in {:?}", result.status, result.duration),
    });

    crate::tui::run_tui(rx, app)
        .await
        .map_err(|e| anyhow::anyhow!("TUI error: {e}"))?;

    Ok(exit_code_for_status(&result.status))
}
