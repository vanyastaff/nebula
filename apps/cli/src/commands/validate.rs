use std::process::ExitCode;

use anyhow::Context;

use crate::cli::{ValidateArgs, resolve_format};
use crate::output;

/// Execute the `validate` command.
pub fn execute(args: ValidateArgs, quiet: bool) -> anyhow::Result<ExitCode> {
    let content = std::fs::read_to_string(&args.workflow)
        .with_context(|| format!("failed to read {}", args.workflow.display()))?;

    let definition = parse_workflow(&content, &args.workflow)?;

    let errors = nebula_workflow::validate_workflow(&definition);
    let error_strings: Vec<String> = errors.iter().map(ToString::to_string).collect();

    if !quiet {
        let format = resolve_format(args.format);
        output::print_validation(&error_strings, &format);
    }

    if errors.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(super::exit_codes::VALIDATION_FAILED))
    }
}

/// Parse a workflow definition from a file, auto-detecting JSON vs YAML by extension.
pub fn parse_workflow(
    content: &str,
    path: &std::path::Path,
) -> anyhow::Result<nebula_workflow::WorkflowDefinition> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");

    match ext {
        "yaml" | "yml" => {
            serde_yaml::from_str(content).with_context(|| "failed to parse YAML workflow")
        }
        "json" => serde_json::from_str(content).with_context(|| "failed to parse JSON workflow"),
        other => {
            // Try YAML first, fall back to JSON.
            serde_yaml::from_str::<nebula_workflow::WorkflowDefinition>(content)
                .or_else(|_| serde_json::from_str(content))
                .with_context(|| format!("failed to parse workflow with extension .{other}"))
        }
    }
}
