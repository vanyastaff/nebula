use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use nebula_core::id::ExecutionId;
use nebula_engine::WorkflowEngine;
use nebula_execution::ExecutionStatus;
use nebula_execution::context::ExecutionBudget;
use nebula_execution::plan::ExecutionPlan;
use nebula_runtime::{ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox};
use nebula_telemetry::metrics::MetricsRegistry;

use crate::cli::{OutputFormat, RunArgs, resolve_format};

/// Execute the `run` command.
pub async fn execute(args: RunArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    // 1. Load and parse workflow.
    let content = std::fs::read_to_string(&args.workflow)
        .with_context(|| format!("failed to read {}", args.workflow.display()))?;

    let mut definition = super::validate::parse_workflow_lenient(&content, &args.workflow)?;

    // 2. Apply --set overrides.
    if !args.overrides.is_empty() {
        apply_overrides(&mut definition, &args.overrides)?;
    }

    // 3. Validate workflow.
    let errors = nebula_workflow::validate_workflow(&definition);
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("validation error: {err}");
        }
        return Ok(ExitCode::from(super::exit_codes::VALIDATION_FAILED));
    }

    // 3. Dry-run: show execution plan and exit.
    if args.dry_run {
        return dry_run(&definition, &args, quiet);
    }

    // 4. Parse input data (from --input or --input-file).
    let input: serde_json::Value = load_input(&args)?;

    // 5. Build the execution stack.
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

    // 4. Build execution budget.
    let budget = ExecutionBudget {
        max_concurrent_nodes: args.concurrency,
        max_duration: args.timeout,
        max_output_bytes: None,
        max_total_retries: None,
    };

    // 7. Optionally attach event channel for --stream or --tui.
    let want_events = args.stream
        || cfg!(feature = "tui") && {
            #[cfg(feature = "tui")]
            {
                args.tui
            }
            #[cfg(not(feature = "tui"))]
            {
                false
            }
        };

    let (event_tx, mut event_rx) = if want_events {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let mut engine = WorkflowEngine::new(runtime, metrics);
    if let Some(tx) = event_tx {
        engine = engine.with_event_sender(tx);
    }

    // TUI mode: run engine in background, TUI consumes live events.
    #[cfg(feature = "tui")]
    if args.tui {
        return run_tui_live(&definition, engine, input, budget, event_rx.unwrap()).await;
    }

    let result = engine.execute_workflow(&definition, input, budget).await?;

    // --stream: print collected node events to stderr.
    if args.stream
        && !quiet
        && let Some(ref mut rx) = event_rx
    {
        while let Ok(event) = rx.try_recv() {
            print_stream_event(&event);
        }
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
        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
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
                let end = json.char_indices().nth(200).map_or(json.len(), |(i, _)| i);
                format!("{}...", &json[..end])
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

/// Apply --set overrides to workflow node parameters.
///
/// Format: `<node_name>.params.<key>=<value>`
/// Example: `fetch.params.url=https://staging.api.com`
fn apply_overrides(
    workflow: &mut nebula_workflow::WorkflowDefinition,
    overrides: &[String],
) -> anyhow::Result<()> {
    let node_names: Vec<String> = workflow.nodes.iter().map(|n| n.name.clone()).collect();

    for override_str in overrides {
        let (path, value) = override_str
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid --set format: \"{override_str}\"\nExpected: <node_name>.params.<key>=<value>"))?;

        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() < 3 || parts[1] != "params" {
            anyhow::bail!("invalid --set path: \"{path}\"\nExpected: <node_name>.params.<key>");
        }

        let node_name = parts[0];
        let param_key = parts[2..].join(".");

        // Find node by name (case-insensitive).
        let node = workflow
            .nodes
            .iter_mut()
            .find(|n| n.name.eq_ignore_ascii_case(node_name));

        let node = match node {
            Some(n) => n,
            None => {
                let suggestion = find_closest(&node_names, node_name);
                let hint = suggestion
                    .map(|s| format!(" Did you mean \"{s}\"?"))
                    .unwrap_or_default();
                anyhow::bail!(
                    "unknown node \"{node_name}\" in --set.{hint}\nAvailable: {}",
                    node_names.join(", ")
                );
            }
        };

        // Parse value as JSON, fall back to string.
        let json_value: serde_json::Value = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));

        // Set as a literal parameter.
        node.parameters.insert(
            param_key,
            nebula_workflow::ParamValue::Literal { value: json_value },
        );
    }

    Ok(())
}

/// Find the closest match to `target` in `options` (simple Levenshtein-like).
fn find_closest<'a>(options: &'a [String], target: &str) -> Option<&'a str> {
    let target_lower = target.to_lowercase();
    options
        .iter()
        .filter(|o| {
            let o_lower = o.to_lowercase();
            // Simple heuristic: starts with same prefix or edit distance < 3
            o_lower.starts_with(&target_lower[..target_lower.len().clamp(1, 3)])
                || o_lower.contains(&target_lower)
        })
        .min_by_key(|o| {
            // Crude edit distance approximation
            let o_lower = o.to_lowercase();
            if o_lower == target_lower {
                return 0;
            }
            if o_lower.contains(&target_lower) || target_lower.contains(&o_lower) {
                return 1;
            }
            o_lower.len().abs_diff(target_lower.len()) + 2
        })
        .map(|s| s.as_str())
}

/// --dry-run: show execution plan without running.
fn dry_run(
    workflow: &nebula_workflow::WorkflowDefinition,
    args: &RunArgs,
    quiet: bool,
) -> anyhow::Result<ExitCode> {
    let budget = ExecutionBudget {
        max_concurrent_nodes: args.concurrency,
        max_duration: args.timeout,
        max_output_bytes: None,
        max_total_retries: None,
    };

    let plan = ExecutionPlan::from_workflow(ExecutionId::new(), workflow, budget)
        .map_err(|e| anyhow::anyhow!("failed to build execution plan: {e}"))?;

    if quiet {
        return Ok(ExitCode::SUCCESS);
    }

    let format = resolve_format(args.format.clone());
    match format {
        OutputFormat::Json => {
            let groups: Vec<Vec<String>> = plan
                .parallel_groups
                .iter()
                .map(|g| g.iter().map(ToString::to_string).collect())
                .collect();
            let json = serde_json::json!({
                "total_nodes": plan.total_nodes,
                "entry_nodes": plan.entry_nodes.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "exit_nodes": plan.exit_nodes.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "parallel_groups": groups,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
            );
        }
        OutputFormat::Text => {
            println!("Execution Plan (dry-run)");
            println!("  Nodes:        {}", plan.total_nodes);
            println!("  Entry points: {}", plan.entry_nodes.len());
            println!("  Exit points:  {}", plan.exit_nodes.len());
            println!("  Concurrency:  {}", args.concurrency);
            println!();

            // Show node details per group.
            for (i, group) in plan.parallel_groups.iter().enumerate() {
                println!("  Level {}:", i + 1);
                for node_id in group {
                    let name = workflow
                        .nodes
                        .iter()
                        .find(|n| n.id == *node_id)
                        .map(|n| format!("{} ({})", n.name, n.action_key))
                        .unwrap_or_else(|| node_id.to_string());
                    println!("    {node_id}  {name}");
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Print a single engine event to stderr (for --stream mode).
fn print_stream_event(event: &nebula_engine::ExecutionEvent) {
    use nebula_engine::ExecutionEvent;
    match event {
        ExecutionEvent::NodeStarted {
            node_id,
            action_key,
            ..
        } => eprintln!("  ▶ {node_id} ({action_key}) started"),
        ExecutionEvent::NodeCompleted {
            node_id, elapsed, ..
        } => eprintln!("  ✓ {node_id} completed ({elapsed:?})"),
        ExecutionEvent::NodeFailed { node_id, error, .. } => {
            eprintln!("  ✗ {node_id} failed: {error}")
        }
        ExecutionEvent::NodeSkipped { node_id, .. } => {
            eprintln!("  ⊘ {node_id} skipped");
        }
        ExecutionEvent::ExecutionFinished {
            success, elapsed, ..
        } => {
            let status = if *success { "completed" } else { "failed" };
            eprintln!("  ═ Execution {status} ({elapsed:?})");
        }
        _ => {}
    }
}

fn exit_code_for_status(status: &ExecutionStatus) -> ExitCode {
    match status {
        ExecutionStatus::Completed => ExitCode::SUCCESS,
        ExecutionStatus::TimedOut => ExitCode::from(super::exit_codes::TIMEOUT),
        _ => ExitCode::from(super::exit_codes::WORKFLOW_FAILED),
    }
}

/// Launch the TUI with engine events collected during execution.
///
/// Engine runs first (events sent via channel), then TUI displays
/// the collected events + final result interactively.
#[cfg(feature = "tui")]
async fn run_tui_live(
    workflow: &nebula_workflow::WorkflowDefinition,
    engine: WorkflowEngine,
    input: serde_json::Value,
    budget: ExecutionBudget,
    mut engine_rx: tokio::sync::mpsc::UnboundedReceiver<nebula_engine::ExecutionEvent>,
) -> anyhow::Result<ExitCode> {
    use crate::tui::app::App;
    use crate::tui::event::{LogLevel, TuiEvent};

    // Run engine — events are sent synchronously during execution.
    let result = engine.execute_workflow(workflow, input, budget).await?;

    // Build TUI app from workflow definition.
    let node_order: Vec<_> = workflow
        .nodes
        .iter()
        .map(|n| (n.id, n.name.clone(), n.action_key.to_string()))
        .collect();

    let (_tui_tx, tui_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        workflow.name.clone(),
        result.execution_id.to_string(),
        node_order,
    );

    // Replay collected engine events into the TUI app state.
    while let Ok(event) = engine_rx.try_recv() {
        let tui_event = match event {
            nebula_engine::ExecutionEvent::NodeStarted {
                node_id,
                action_key,
                ..
            } => TuiEvent::NodeStarted {
                node_id,
                name: String::new(),
                action_key,
            },
            nebula_engine::ExecutionEvent::NodeCompleted {
                node_id, elapsed, ..
            } => TuiEvent::NodeCompleted {
                node_id,
                elapsed,
                output: serde_json::Value::Null,
            },
            nebula_engine::ExecutionEvent::NodeFailed { node_id, error, .. } => {
                TuiEvent::NodeFailed {
                    node_id,
                    elapsed: std::time::Duration::ZERO,
                    error,
                }
            }
            nebula_engine::ExecutionEvent::NodeSkipped { node_id, .. } => TuiEvent::Log {
                level: LogLevel::Info,
                message: format!("node {node_id} skipped"),
            },
            nebula_engine::ExecutionEvent::ExecutionFinished {
                success, elapsed, ..
            } => TuiEvent::WorkflowDone {
                total_elapsed: elapsed,
                success,
            },
            _ => continue,
        };
        app.apply_event(tui_event);
    }

    // Also populate outputs/errors from result (events don't carry output data).
    for (node_id, output) in &result.node_outputs {
        if let Some(&idx) = app.node_index.get(node_id) {
            app.nodes[idx].1.output = Some(output.clone());
        }
    }
    for (node_id, error) in &result.node_errors {
        if let Some(&idx) = app.node_index.get(node_id) {
            app.nodes[idx].1.error = Some(error.clone());
        }
    }

    app.done = true;
    app.success = result.status == ExecutionStatus::Completed;

    crate::tui::run_tui(tui_rx, app)
        .await
        .map_err(|e| anyhow::anyhow!("TUI error: {e}"))?;

    Ok(exit_code_for_status(&result.status))
}
