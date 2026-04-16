use std::{sync::Arc, time::Duration};

use nebula_action::{
    ActionContext, ActionHandler, ActionResult, BreakReason, TriggerContext, testing::SpyEmitter,
};
use nebula_core::{
    id::{ExecutionId, WorkflowId},
    node_key,
};
use nebula_runtime::ActionRegistry;
use tokio_util::sync::CancellationToken;

use crate::cli::{ActionsInfoArgs, ActionsListArgs, ActionsTestArgs, OutputFormat, resolve_format};

/// Maximum iterations for stateful actions to guard against runaway loops.
const STATEFUL_MAX_ITERATIONS: u32 = 1000;

/// Default window for trigger smoke-tests (read via `--input.timeout_ms`).
const TRIGGER_DEFAULT_TIMEOUT_MS: u64 = 2000;

fn build_registry() -> Arc<ActionRegistry> {
    Arc::new(ActionRegistry::new())
}

/// Execute the `actions list` command.
pub fn list(args: ActionsListArgs) {
    let registry = build_registry();
    let mut keys = registry.keys();
    keys.sort();

    let format = resolve_format(args.format);

    match format {
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = keys
                .iter()
                .filter_map(|k| registry.get(k))
                .map(|(meta, _)| {
                    serde_json::json!({
                        "key": meta.key.as_str(),
                        "name": meta.name,
                        "description": meta.description,
                        "version": meta.version.to_string(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries).expect("json"));
        },
        OutputFormat::Text => {
            let header = format!("{:<12} {:<10} {:<8} DESCRIPTION", "KEY", "NAME", "VER");
            println!("{header}");
            println!("{}", "-".repeat(64));
            for key in &keys {
                if let Some((meta, _)) = registry.get(key) {
                    println!(
                        "{:<12} {:<10} {:<8} {}",
                        meta.key.as_str(),
                        meta.name,
                        meta.version,
                        meta.description,
                    );
                }
            }
        },
    }
}

/// Execute the `actions info` command.
pub fn info(args: ActionsInfoArgs) {
    let registry = build_registry();
    let format = resolve_format(args.format);

    match registry.get_by_str(&args.key) {
        Some((meta, _)) => match format {
            OutputFormat::Json => {
                let json = serde_json::json!({
                    "key": meta.key.as_str(),
                    "name": meta.name,
                    "version": meta.version.to_string(),
                    "description": meta.description,
                    "isolation": format!("{:?}", meta.isolation_level),
                });
                println!("{}", serde_json::to_string_pretty(&json).expect("json"));
            },
            OutputFormat::Text => {
                println!("Key:         {}", meta.key.as_str());
                println!("Name:        {}", meta.name);
                println!("Version:     {}", meta.version);
                println!("Description: {}", meta.description);
                println!("Isolation:   {:?}", meta.isolation_level);
                println!(
                    "Parameters:  {}",
                    if meta.parameters.is_empty() {
                        "none"
                    } else {
                        "(defined)"
                    }
                );
            },
        },
        None => {
            eprintln!("error: action '{}' not found", args.key);
            eprintln!();
            eprintln!("Available actions:");
            let mut keys = registry.keys();
            keys.sort();
            for key in &keys {
                eprintln!("  {key}");
            }
            std::process::exit(1);
        },
    }
}

/// Execute the `actions test` command.
///
/// Dispatches on the `ActionHandler` variant so the same `--input` JSON
/// works for any action kind:
/// - `Stateless` → one execute call
/// - `Stateful` → iterative loop (init_state → execute until `Break` or cap)
/// - other variants → not yet supported
pub async fn test(args: ActionsTestArgs) {
    let registry = build_registry();
    let format = resolve_format(args.format);

    let (meta, handler) = match registry.get_by_str(&args.key) {
        Some(entry) => entry,
        None => {
            eprintln!("error: action '{}' not found", args.key);
            let mut keys = registry.keys();
            keys.sort();
            let names: Vec<String> = keys.iter().map(|k| k.as_str().to_owned()).collect();
            eprintln!("Available: {}", names.join(", "));
            std::process::exit(1);
        },
    };

    let input: serde_json::Value = match serde_json::from_str(&args.input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: invalid --input JSON: {e}");
            std::process::exit(1);
        },
    };

    // Build a minimal ActionContext.
    let ctx = ActionContext::new(
        ExecutionId::new(),
        node_key!("test"),
        WorkflowId::new(),
        CancellationToken::new(),
    );

    eprintln!("Testing: {} ({})", meta.name, meta.key);
    eprintln!(
        "Input:   {}",
        serde_json::to_string(&input).unwrap_or_default()
    );
    eprintln!();

    let start = std::time::Instant::now();
    let outcome = match &handler {
        ActionHandler::Stateless(h) => run_stateless(h.as_ref(), input, &ctx).await,
        ActionHandler::Stateful(h) => run_stateful(h.as_ref(), input, &ctx).await,
        ActionHandler::Trigger(h) => run_trigger(h.clone(), &input).await,
        other => {
            eprintln!("error: `actions test` does not yet support this action kind: {other:?}");
            std::process::exit(2);
        },
    };
    let elapsed = start.elapsed();

    match outcome {
        Ok(report) => print_report(&report, elapsed, format),
        Err(e) => {
            eprintln!("Status:   FAILED");
            eprintln!("Duration: {elapsed:?}");
            eprintln!("Error:    {e}");
            std::process::exit(2);
        },
    }
}

/// Structured result of a test run, used by the output formatter.
struct TestReport {
    output: serde_json::Value,
    iterations: u32,
    kind: &'static str,
    /// Optional trailing note (e.g. break reason, truncation warning).
    note: Option<String>,
}

async fn run_stateless(
    handler: &dyn nebula_action::StatelessHandler,
    input: serde_json::Value,
    ctx: &ActionContext,
) -> Result<TestReport, nebula_action::ActionError> {
    let result = handler.execute(input, ctx).await?;
    let output = extract_output(&result);
    Ok(TestReport {
        output,
        iterations: 1,
        kind: "stateless",
        note: None,
    })
}

async fn run_stateful(
    handler: &dyn nebula_action::StatefulHandler,
    input: serde_json::Value,
    ctx: &ActionContext,
) -> Result<TestReport, nebula_action::ActionError> {
    let mut state = handler.init_state()?;
    let mut iterations = 0u32;

    loop {
        iterations += 1;
        let result = handler.execute(&input, &mut state, ctx).await?;
        let last_output = extract_output(&result);

        match &result {
            ActionResult::Continue { progress, .. } => {
                let pct = progress
                    .map(|p| format!("{:>5.1}%", p * 100.0))
                    .unwrap_or_else(|| "  ?  ".to_owned());
                eprintln!(
                    "  iter {iterations:>3}: Continue ({pct})  partial={}",
                    truncate(&compact_json(&last_output), 160)
                );
                if iterations >= STATEFUL_MAX_ITERATIONS {
                    return Ok(TestReport {
                        output: last_output,
                        iterations,
                        kind: "stateful",
                        note: Some(format!(
                            "hit iteration cap ({STATEFUL_MAX_ITERATIONS}) — stopping"
                        )),
                    });
                }
            },
            ActionResult::Break { reason, .. } => {
                eprintln!(
                    "  iter {iterations:>3}: Break ({}) final={}",
                    format_break_reason(reason),
                    truncate(&compact_json(&last_output), 160)
                );
                return Ok(TestReport {
                    output: last_output,
                    iterations,
                    kind: "stateful",
                    note: Some(format!("break: {}", format_break_reason(reason))),
                });
            },
            // Any other variant is a non-iterative outcome — surface and stop.
            other => {
                return Ok(TestReport {
                    output: last_output,
                    iterations,
                    kind: "stateful",
                    note: Some(format!("non-iterative result: {other:?}")),
                });
            },
        }
    }
}

/// Drive a trigger for a bounded time window and collect emitted executions.
///
/// Builds a `TriggerContext` with a `SpyEmitter` hooked in, spawns
/// `handler.start()` in the background, waits `timeout_ms` (from
/// `--input.timeout_ms`, default 2s), cancels the token, awaits start to
/// return, then reports everything the trigger pushed to the emitter.
async fn run_trigger(
    handler: Arc<dyn nebula_action::TriggerHandler>,
    input: &serde_json::Value,
) -> Result<TestReport, nebula_action::ActionError> {
    let timeout_ms = input
        .get("timeout_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(TRIGGER_DEFAULT_TIMEOUT_MS);

    let cancel = CancellationToken::new();
    let spy = Arc::new(SpyEmitter::new());
    let ctx = TriggerContext::new(WorkflowId::new(), node_key!("test"), cancel.clone())
        .with_emitter(spy.clone());

    eprintln!("  trigger window: {timeout_ms}ms");

    // Spawn the trigger's run loop. `start()` blocks until the token is cancelled
    // (or it exits on its own).
    let start_handle = {
        let handler = handler.clone();
        let ctx = ctx.clone();
        tokio::spawn(async move { handler.start(&ctx).await })
    };

    // Let the trigger run for the configured window, then cancel.
    tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
    cancel.cancel();

    // Await the start loop with a small grace period — if it doesn't exit,
    // we stop caring and just surface whatever the spy collected.
    let start_result = match tokio::time::timeout(Duration::from_secs(5), start_handle).await {
        Ok(Ok(res)) => res,
        Ok(Err(join_err)) => Err(nebula_action::ActionError::fatal(format!(
            "trigger start panicked: {join_err}"
        ))),
        Err(_) => {
            eprintln!("  warning: trigger did not exit within grace period");
            Ok(())
        },
    };

    // Best-effort stop — most adapters are no-ops here (cancellation already
    // killed the loop), but give them a chance to release state.
    let _ = handler.stop(&ctx).await;

    let emitted = spy.emitted();
    let count = emitted.len();

    eprintln!("  emitted {count} execution(s)");

    let note = match (&start_result, count) {
        (Err(e), _) => Some(format!("start() returned error: {e}")),
        (Ok(()), 0) => Some("no events emitted in window".to_owned()),
        (Ok(()), n) => Some(format!("{n} execution(s) emitted")),
    };

    Ok(TestReport {
        output: serde_json::Value::Array(emitted),
        iterations: count as u32,
        kind: "trigger",
        note,
    })
}

fn extract_output(result: &ActionResult<serde_json::Value>) -> serde_json::Value {
    match result {
        ActionResult::Success { output }
        | ActionResult::Continue { output, .. }
        | ActionResult::Break { output, .. } => output
            .as_value()
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        other => serde_json::json!(format!("{other:?}")),
    }
}

fn format_break_reason(reason: &BreakReason) -> String {
    match reason {
        BreakReason::Completed => "Completed".to_owned(),
        BreakReason::MaxIterations => "MaxIterations".to_owned(),
        BreakReason::ConditionMet => "ConditionMet".to_owned(),
        BreakReason::Custom(s) => format!("Custom({s})"),
        other => format!("{other:?}"),
    }
}

fn compact_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "<non-serializable>".to_owned())
}

/// Truncate `s` to `max` characters, appending `...` when cut.
/// Safe against splitting multibyte UTF-8: walks char boundaries.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn print_report(report: &TestReport, elapsed: std::time::Duration, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json = serde_json::json!({
                "status": "ok",
                "kind": report.kind,
                "iterations": report.iterations,
                "output": report.output,
                "duration_ms": elapsed.as_millis(),
                "note": report.note,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
            );
        },
        OutputFormat::Text => {
            println!("Status:     ok");
            println!("Kind:       {}", report.kind);
            println!("Iterations: {}", report.iterations);
            println!("Duration:   {elapsed:?}");
            if let Some(note) = &report.note {
                println!("Note:       {note}");
            }
            println!(
                "Output:     {}",
                serde_json::to_string_pretty(&report.output).unwrap_or_else(|_| "null".to_owned())
            );
        },
    }
}
