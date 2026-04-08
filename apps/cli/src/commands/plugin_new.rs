use std::fs;
use std::path::Path;

use anyhow::{Context, bail};
use nebula_core::{ActionKey, PluginKey};

use crate::cli::PluginNewArgs;

/// Execute the `plugin new` command.
pub fn execute(args: PluginNewArgs) -> anyhow::Result<()> {
    let name = &args.name;

    // Validate plugin name as a valid PluginKey.
    let plugin_key = name.replace('-', "_");
    PluginKey::new(&plugin_key).map_err(|e| {
        anyhow::anyhow!(
            "invalid plugin name \"{name}\": {e}\n\
             Allowed: a-z, 0-9, underscore, dot, dash. Must end with alphanumeric."
        )
    })?;

    let dir = args
        .path
        .unwrap_or_else(|| format!("nebula-plugin-{name}").into());

    if dir.exists() {
        bail!("directory {} already exists", dir.display());
    }

    let crate_name = format!("nebula-plugin-{name}");
    let struct_name = to_pascal_case(name);

    // Create directories.
    fs::create_dir_all(dir.join("src"))
        .with_context(|| format!("failed to create {}/src", dir.display()))?;

    // Generate and validate action names.
    let action_names: Vec<String> = if args.actions == 1 {
        vec!["execute".to_owned()]
    } else {
        (1..=args.actions).map(|i| format!("action_{i}")).collect()
    };

    // Validate each full action key (plugin_key.action_name).
    for action in &action_names {
        let full_key = format!("{plugin_key}.{action}");
        ActionKey::new(&full_key)
            .map_err(|e| anyhow::anyhow!("invalid action key \"{full_key}\": {e}"))?;
    }

    // Write files.
    write_file(&dir.join("Cargo.toml"), &cargo_toml(&crate_name, name))?;
    write_file(
        &dir.join("src/main.rs"),
        &main_rs(&plugin_key, &struct_name, name, &action_names),
    )?;
    write_file(
        &dir.join("README.md"),
        &readme(name, &plugin_key, &action_names),
    )?;

    // Print summary.
    println!("Created plugin: {}", dir.display());
    println!();
    println!("  {}/", dir.display());
    println!("  ├── Cargo.toml");
    println!("  ├── src/");
    println!("  │   └── main.rs");
    println!("  └── README.md");
    println!();
    println!("Plugin key:  {plugin_key}");
    println!("Actions:");
    for action in &action_names {
        println!("  {plugin_key}.{action}");
    }
    println!();
    println!("Next steps:");
    println!("  cd {}", dir.display());
    println!("  cargo build");
    println!("  cp target/debug/{crate_name} ~/.local/share/nebula/plugins/");
    println!("  nebula plugin list");

    Ok(())
}

fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn to_pascal_case(s: &str) -> String {
    s.split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + chars.as_str()
                }
                None => String::new(),
            }
        })
        .collect()
}

fn cargo_toml(crate_name: &str, name: &str) -> String {
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"
rust-version = "1.94"
description = "Nebula plugin: {name}"
license = "MIT OR Apache-2.0"

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
    )
}

fn main_rs(plugin_key: &str, struct_name: &str, name: &str, action_names: &[String]) -> String {
    let action_descriptors: String = action_names
        .iter()
        .map(|a| {
            let display_name = to_pascal_case(a);
            format!(
                r#"            {{
                "key": "{plugin_key}.{a}",
                "name": "{display_name}",
                "description": "TODO: describe {a}"
            }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    let action_match_arms: String = action_names
        .iter()
        .map(|a| {
            format!(
                r#"        "{plugin_key}.{a}" | "{a}" => {{
            // TODO: implement {a}
            Ok(serde_json::json!({{"ok": true, "action": "{a}"}}))
        }}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"//! Nebula plugin: {name}
//!
//! Protocol: reads JSON from stdin, writes JSON to stdout.
//! - `{{"action_key": "__metadata__"}}` → returns plugin metadata
//! - `{{"action_key": "...", "input": {{...}}}}` → executes action

use serde_json::{{Value, json}};

fn main() {{
    let request: Value = serde_json::from_reader(std::io::stdin()).unwrap_or(json!({{}}));
    let action_key = request["action_key"].as_str().unwrap_or("");
    let input = &request["input"];

    let result = handle(action_key, input);

    match result {{
        Ok(output) => println!("{{}}", json!({{"output": output}})),
        Err(e) => println!("{{}}", json!({{"error": e, "code": "PLUGIN_ERROR"}})),
    }}
}}

fn handle(action_key: &str, input: &Value) -> Result<Value, String> {{
    match action_key {{
        "__metadata__" => Ok(json!({{
            "key": "{plugin_key}",
            "name": "{struct_name}",
            "version": 1,
            "description": "TODO: describe {name} plugin",
            "actions": [
{action_descriptors}
            ]
        }})),
{action_match_arms}
        other => Err(format!("unknown action: {{other}}")),
    }}
}}
"#
    )
}

fn readme(name: &str, plugin_key: &str, action_names: &[String]) -> String {
    let actions_list: String = action_names
        .iter()
        .map(|a| format!("- `{plugin_key}.{a}`"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# nebula-plugin-{name}

Nebula community plugin: {name}

## Actions

{actions_list}

## Build & Install

```bash
cargo build --release
cp target/release/nebula-plugin-{name} ~/.local/share/nebula/plugins/
nebula plugin list
```

## Test

```bash
echo '{{"action_key":"__metadata__","input":{{}}}}' | cargo run
echo '{{"action_key":"{plugin_key}.{first_action}","input":{{"key":"value"}}}}' | cargo run
```
"#,
        first_action = action_names
            .first()
            .map(|s| s.as_str())
            .unwrap_or("execute"),
    )
}
