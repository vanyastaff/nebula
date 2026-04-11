use std::{fs, path::Path};

use anyhow::{Context, bail};

use crate::cli::DevInitArgs;

/// Execute the `dev init` command.
pub fn execute(args: DevInitArgs) -> anyhow::Result<()> {
    let dir = &args.path;

    let name = args.name.unwrap_or_else(|| {
        dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("my-workflow")
            .to_owned()
    });

    // Create the directory if it doesn't exist.
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create directory {}", dir.display()))?;
    }

    // Don't overwrite existing files.
    let workflow_path = dir.join("workflow.yaml");
    if workflow_path.exists() {
        bail!("{} already exists — aborting", workflow_path.display());
    }

    write_file(&workflow_path, &workflow_template(&name))?;
    write_file(&dir.join(".gitignore"), GITIGNORE)?;

    println!("Initialized Nebula project in {}", dir.display());
    println!();
    println!("  workflow.yaml   — example workflow definition");
    println!("  .gitignore      — ignores build artifacts");
    println!();
    println!("Next steps:");
    println!("  nebula validate workflow.yaml");
    println!("  nebula run workflow.yaml --input '{{\"name\": \"world\"}}'");

    Ok(())
}

fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn workflow_template(name: &str) -> String {
    format!(
        r#"id: "00000000-0000-0000-0000-000000000001"
name: "{name}"
description: "A starter workflow"
version:
  major: 1
  minor: 0
  patch: 0
schema_version: 1
owner_id: "00000000-0000-0000-0000-000000000099"
created_at: "2026-01-01T00:00:00Z"
updated_at: "2026-01-01T00:00:00Z"

nodes:
  - id: "00000000-0000-0000-0000-000000000010"
    name: "Receive Input"
    action_key: "echo"
    parameters: {{}}

  - id: "00000000-0000-0000-0000-000000000020"
    name: "Log Data"
    action_key: "log"
    parameters: {{}}

connections:
  - from_node: "00000000-0000-0000-0000-000000000010"
    to_node: "00000000-0000-0000-0000-000000000020"

config:
  error_strategy: "fail_fast"
"#
    )
}

const GITIGNORE: &str = "# Nebula
.nebula/
*.log
";
