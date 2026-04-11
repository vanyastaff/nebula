use std::{fs, path::Path};

use anyhow::{Context, bail};

use crate::cli::DevActionNewArgs;

/// Execute the `dev action new` command.
pub fn execute(args: DevActionNewArgs) -> anyhow::Result<()> {
    let name = &args.name;
    let dir = args.path.unwrap_or_else(|| name.into());

    if dir.exists() {
        bail!("directory {} already exists", dir.display());
    }

    fs::create_dir_all(dir.join("src"))
        .with_context(|| format!("failed to create {}/src", dir.display()))?;
    fs::create_dir_all(dir.join("tests"))
        .with_context(|| format!("failed to create {}/tests", dir.display()))?;

    let crate_name = format!("nebula-action-{name}");
    let action_key = name.replace('-', "_");
    let struct_name = to_pascal_case(name);

    write_file(&dir.join("Cargo.toml"), &cargo_toml(&crate_name, name))?;
    write_file(&dir.join("src/lib.rs"), &lib_rs(&action_key, &struct_name))?;
    write_file(
        &dir.join("tests/integration.rs"),
        &test_rs(&action_key, &struct_name),
    )?;

    println!("Created action project: {}", dir.display());
    println!();
    println!("  {}/", dir.display());
    println!("  ├── Cargo.toml");
    println!("  ├── src/");
    println!("  │   └── lib.rs       # Action implementation");
    println!("  └── tests/");
    println!("      └── integration.rs");
    println!();
    println!("Action key:  {action_key}");
    println!("Struct name: {struct_name}");
    println!();
    println!("Next steps:");
    println!("  cd {}", dir.display());
    println!("  cargo check");

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
description = "Nebula action: {name}"
license = "MIT OR Apache-2.0"

[dependencies]
nebula-action = {{ version = "0.1" }}
nebula-core = {{ version = "0.1" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
async-trait = "0.1"

[dev-dependencies]
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
"#
    )
}

fn lib_rs(action_key: &str, struct_name: &str) -> String {
    format!(
        r#"//! Nebula action: {action_key}

use nebula_action::context::Context;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::result::ActionResult;
use nebula_action::{{Action, ActionDependencies, StatelessAction}};
use nebula_core::action_key;

/// {struct_name} action.
pub struct {struct_name} {{
    meta: ActionMetadata,
}}

impl {struct_name} {{
    /// Create a new instance.
    #[must_use]
    pub fn new() -> Self {{
        Self {{
            meta: ActionMetadata::new(
                action_key!("{action_key}"),
                "{struct_name}",
                "TODO: describe what this action does",
            ),
        }}
    }}
}}

impl Default for {struct_name} {{
    fn default() -> Self {{
        Self::new()
    }}
}}

impl ActionDependencies for {struct_name} {{}}

impl Action for {struct_name} {{
    fn metadata(&self) -> &ActionMetadata {{
        &self.meta
    }}
}}

impl StatelessAction for {struct_name} {{
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(
        &self,
        input: Self::Input,
        _ctx: &impl Context,
    ) -> Result<ActionResult<Self::Output>, ActionError> {{
        // TODO: implement your action logic here.
        Ok(ActionResult::success(input))
    }}
}}
"#
    )
}

fn test_rs(action_key: &str, struct_name: &str) -> String {
    format!(
        r#"use serde_json::json;

#[tokio::test]
async fn {action_key}_passes_input_through() {{
    // TODO: replace with real test using TestContextBuilder
    // when nebula-action test utilities are available.
    let _ = json!({{ "key": "value" }});
    assert!(true, "{struct_name} action should work");
}}
"#
    )
}
