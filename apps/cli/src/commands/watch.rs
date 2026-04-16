//! Watch a workflow file and re-run on changes.

use std::{
    path::Path,
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use nebula_engine::WorkflowEngine;
use nebula_execution::{ExecutionStatus, context::ExecutionBudget};
use nebula_runtime::{ActionRegistry, ActionRuntime, DataPassingPolicy, InProcessSandbox};
use nebula_sandbox::ActionExecutor;
use nebula_telemetry::metrics::MetricsRegistry;
use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::cli::WatchArgs;

/// Execute the `watch` command.
pub async fn execute(args: WatchArgs) -> anyhow::Result<ExitCode> {
    let workflow_path = args
        .workflow
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", args.workflow.display()))?;

    eprintln!("Watching {} for changes...", workflow_path.display());
    eprintln!("Press Ctrl+C to stop.\n");

    // Run once immediately.
    run_workflow(&workflow_path, &args).await;

    // Watch for file changes.
    let (tx, mut rx) = mpsc::channel::<()>(1);

    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res
            && (event.kind.is_modify() || event.kind.is_create())
        {
            let _ = tx.try_send(());
        }
    })
    .context("failed to create file watcher")?;

    watcher
        .watch(
            workflow_path.parent().unwrap_or(Path::new(".")),
            RecursiveMode::NonRecursive,
        )
        .context("failed to watch directory")?;

    // Debounce: wait 200ms after last change before re-running.
    let mut last_change = Instant::now();

    loop {
        tokio::select! {
            Some(()) = rx.recv() => {
                last_change = Instant::now();
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if last_change.elapsed() < Duration::from_millis(300)
                    && last_change.elapsed() >= Duration::from_millis(200)
                {
                    eprintln!("--- File changed, re-running...\n");
                    run_workflow(&workflow_path, &args).await;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nStopped watching.");
                break;
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

async fn run_workflow(path: &Path, args: &WatchArgs) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error reading file: {e}");
            return;
        },
    };

    let mut definition = match super::validate::parse_workflow_lenient(&content, path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse error: {e}");
            return;
        },
    };

    // Apply --set overrides.
    if !args.overrides.is_empty()
        && let Err(e) = super::run::apply_overrides(&mut definition, &args.overrides)
    {
        eprintln!("override error: {e}");
        return;
    }

    // Validate.
    let errors = nebula_workflow::validate_workflow(&definition);
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("validation: {err}");
        }
        return;
    }

    // Parse input.
    let input: serde_json::Value = match serde_json::from_str(&args.input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("input error: {e}");
            return;
        },
    };

    // Build engine (fresh each run for simplicity).
    let registry = Arc::new(ActionRegistry::new());

    let metrics = MetricsRegistry::new();
    // Sandbox executor is unreachable in Phase 7.5 — see run.rs for details.
    let executor: ActionExecutor = Arc::new(|_ctx, _metadata, _input| {
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
    let budget = ExecutionBudget {
        max_concurrent_nodes: args.concurrency,
        ..Default::default()
    };

    // Execute.
    let start = Instant::now();
    match engine.execute_workflow(&definition, input, budget).await {
        Ok(result) => {
            let elapsed = start.elapsed();
            let status = match result.status {
                ExecutionStatus::Completed => "✓ Completed",
                ExecutionStatus::Failed => "✗ Failed",
                ExecutionStatus::TimedOut => "⏱ Timed out",
                _ => "? Unknown",
            };

            eprintln!(
                "{status} — {} nodes, {elapsed:?}",
                result.node_outputs.len()
            );

            // Show errors if any.
            for (node_key, error) in &result.node_errors {
                eprintln!("  error {node_key}: {error}");
            }

            // Show suggestions.
            let suggestions = crate::suggestions::suggest(&result);
            crate::suggestions::print_suggestions(&suggestions);
        },
        Err(e) => {
            eprintln!("execution error: {e}");
        },
    }

    eprintln!();
}
