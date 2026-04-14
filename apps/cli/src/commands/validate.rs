use std::process::ExitCode;

use anyhow::Context;

use crate::{
    cli::{ValidateArgs, resolve_format},
    output,
};

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

/// Parse a workflow definition strictly (all fields required).
pub fn parse_workflow(
    content: &str,
    path: &std::path::Path,
) -> anyhow::Result<nebula_workflow::WorkflowDefinition> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");

    match ext {
        "yaml" | "yml" => {
            serde_yaml::from_str(content).with_context(|| "failed to parse YAML workflow")
        },
        "json" => serde_json::from_str(content).with_context(|| "failed to parse JSON workflow"),
        _ => serde_yaml::from_str::<nebula_workflow::WorkflowDefinition>(content)
            .or_else(|_| serde_json::from_str(content))
            .with_context(|| "failed to parse workflow"),
    }
}

/// Parse a workflow with auto-filled defaults for local dev convenience.
///
/// Fills missing: `id`, `owner_id`, `created_at`, `updated_at`, `version`, `schema_version`.
/// Uses JSON string roundtrip to satisfy `#[serde(borrow)]` on ActionKey.
pub fn parse_workflow_lenient(
    content: &str,
    path: &std::path::Path,
) -> anyhow::Result<nebula_workflow::WorkflowDefinition> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");

    let mut value: serde_json::Value = match ext {
        "yaml" | "yml" => {
            serde_yaml::from_str(content).with_context(|| "failed to parse YAML workflow")?
        },
        "json" => serde_json::from_str(content).with_context(|| "failed to parse JSON workflow")?,
        _ => serde_yaml::from_str::<serde_json::Value>(content)
            .or_else(|_| serde_json::from_str(content))
            .with_context(|| "failed to parse workflow")?,
    };

    fill_workflow_defaults(&mut value);

    // Roundtrip through JSON string — required because ActionKey uses #[serde(borrow)].
    let json_str = serde_json::to_string(&value)?;
    serde_json::from_str(&json_str).with_context(|| "failed to deserialize workflow")
}

/// Fill default values for fields that are boilerplate in local dev.
fn fill_workflow_defaults(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    if !obj.contains_key("id") {
        obj.insert(
            "id".to_owned(),
            serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
        );
    }
    if !obj.contains_key("owner_id") {
        obj.insert(
            "owner_id".to_owned(),
            serde_json::Value::String("00000000-0000-0000-0000-000000000000".to_owned()),
        );
    }
    if !obj.contains_key("version") {
        obj.insert(
            "version".to_owned(),
            serde_json::json!({"major": 1, "minor": 0, "patch": 0}),
        );
    }
    if !obj.contains_key("schema_version") {
        obj.insert("schema_version".to_owned(), serde_json::json!(1));
    }
    let now = chrono::Utc::now().to_rfc3339();
    if !obj.contains_key("created_at") {
        obj.insert(
            "created_at".to_owned(),
            serde_json::Value::String(now.clone()),
        );
    }
    if !obj.contains_key("updated_at") {
        obj.insert("updated_at".to_owned(), serde_json::Value::String(now));
    }
}
