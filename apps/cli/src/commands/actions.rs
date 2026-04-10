use std::sync::Arc;

use nebula_action::{ActionContext, ActionHandler};
use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use nebula_runtime::ActionRegistry;
use tokio_util::sync::CancellationToken;

use crate::cli::{ActionsInfoArgs, ActionsListArgs, ActionsTestArgs, OutputFormat, resolve_format};

fn build_registry() -> Arc<ActionRegistry> {
    let registry = Arc::new(ActionRegistry::new());
    crate::actions::register_builtins(&registry);
    registry
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
        }
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
        }
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
            }
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
            }
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
        }
    }
}

/// Execute the `actions test` command.
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
        }
    };

    let input: serde_json::Value = match serde_json::from_str(&args.input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: invalid --input JSON: {e}");
            std::process::exit(1);
        }
    };

    // Build a minimal ActionContext.
    let ctx = ActionContext::new(
        ExecutionId::new(),
        NodeId::new(),
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
    let result = match &handler {
        ActionHandler::Stateless(h) => h.execute(input, &ctx).await,
        other => {
            eprintln!(
                "error: `actions test` only supports stateless actions (got {:?})",
                other
            );
            std::process::exit(2);
        }
    };
    let elapsed = start.elapsed();

    match result {
        Ok(action_result) => {
            // Extract primary output from the result.
            let output = match &action_result {
                nebula_action::ActionResult::Success { output } => output
                    .as_value()
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                other => serde_json::json!(format!("{other:?}")),
            };

            match format {
                OutputFormat::Json => {
                    let json = serde_json::json!({
                        "status": "ok",
                        "output": output,
                        "duration_ms": elapsed.as_millis(),
                    });
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
                    );
                }
                OutputFormat::Text => {
                    println!("Status:   ok");
                    println!("Duration: {elapsed:?}");
                    println!(
                        "Output:   {}",
                        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "null".to_owned())
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("Status:   FAILED");
            eprintln!("Duration: {elapsed:?}");
            eprintln!("Error:    {e}");
            std::process::exit(2);
        }
    }
}
