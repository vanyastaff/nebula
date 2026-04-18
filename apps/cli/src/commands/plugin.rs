use std::time::Duration;

use nebula_sandbox::discovery;

use crate::plugins;

/// Execute the `plugin list` command.
pub async fn list() {
    let dirs = plugins::plugin_directories();
    let dir_str = if dirs.is_empty() {
        "(no plugin directories found)".to_owned()
    } else {
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };

    println!("Plugin directories: {dir_str}");
    println!();

    let mut total = 0;

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }

        let plugins = discovery::discover_directory(dir, Duration::from_secs(5)).await;

        for (name, handlers) in &plugins {
            println!("  {name}");
            for (meta, _handler) in handlers {
                println!(
                    "    action: {:<30} {}",
                    meta.base.key.as_str(),
                    meta.base.description
                );
            }
            total += handlers.len();
            println!();
        }
    }

    if total == 0 {
        println!("No community plugins found.");
        println!();
        println!("To add plugins, place binary files in:");
        println!("  ./plugins/  (project-local)");
        if let Some(data) = dirs::data_dir() {
            println!(
                "  {}  (global)",
                data.join("nebula").join("plugins").display()
            );
        }
        println!();
        println!("Binary naming: nebula-plugin-<name> (e.g., nebula-plugin-telegram)");
    } else {
        println!("{total} action(s) from community plugins.");
    }
}
