use crate::cli::OutputFormat;

/// Print a serializable value in the requested format.
pub fn print_json<T: serde::Serialize>(value: &T) {
    let json = serde_json::to_string_pretty(value).expect("failed to serialize output");
    println!("{json}");
}

/// Print a validation result summary.
pub fn print_validation(errors: &[String], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let result = serde_json::json!({
                "valid": errors.is_empty(),
                "errors": errors,
            });
            print_json(&result);
        },
        OutputFormat::Text => {
            if errors.is_empty() {
                println!("Workflow is valid.");
            } else {
                eprintln!("Validation failed with {} error(s):", errors.len());
                for (i, err) in errors.iter().enumerate() {
                    eprintln!("  {}. {err}", i + 1);
                }
            }
        },
    }
}
