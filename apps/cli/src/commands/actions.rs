use std::sync::Arc;

use nebula_runtime::ActionRegistry;

use crate::cli::{ActionsInfoArgs, ActionsListArgs, OutputFormat, resolve_format};

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
                .filter_map(|k| registry.get(k).ok())
                .map(|h| {
                    let meta = h.metadata();
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
                if let Ok(handler) = registry.get(key) {
                    let meta = handler.metadata();
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

    match registry.get(&args.key) {
        Ok(handler) => {
            let meta = handler.metadata();
            match format {
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
            }
        }
        Err(_) => {
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
