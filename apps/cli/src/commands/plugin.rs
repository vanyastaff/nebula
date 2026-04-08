use nebula_plugin::PluginRegistry;

use crate::plugins;

/// Execute the `plugin list` command.
pub fn list() {
    let mut registry = PluginRegistry::new();
    let count = plugins::load_plugins(&mut registry);

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

    if count == 0 {
        println!("No external plugins loaded.");
        println!();
        println!("To install plugins, place .so/.dll/.dylib files in:");
        println!("  ./plugins/  (project-local)");
        if let Some(data) = dirs::data_dir() {
            println!(
                "  {}  (global)",
                data.join("nebula").join("plugins").display()
            );
        }
        println!();
        println!("File naming: nebula_<name>.so (e.g., nebula_slack.so)");
        return;
    }

    println!("{count} plugin(s) loaded:");
    println!();

    for key in registry.keys() {
        let Ok(plugin_type) = registry.get(&key) else {
            continue;
        };
        let Ok(plugin) = plugin_type.get_plugin(None) else {
            continue;
        };

        println!("  {:<20} v{}  {}", plugin.name(), plugin.version(), key);
        print_descriptors(&*plugin);
        println!();
    }
}

fn print_descriptors(plugin: &dyn nebula_plugin::Plugin) {
    for a in &plugin.actions() {
        println!("    action: {} ({})", a.key, a.name);
    }
    for c in &plugin.credentials() {
        println!("    credential: {} ({})", c.key, c.name);
    }
    for r in &plugin.resources() {
        println!("    resource: {} ({})", r.key, r.name);
    }
}
