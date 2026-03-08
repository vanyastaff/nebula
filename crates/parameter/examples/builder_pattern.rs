//! Example: Building a parameter schema with improved DX.

use nebula_parameter::{param_def, param_values, prelude::*};

fn main() {
    // ── Example 1: Using builder pattern with type-specific methods ────────

    let api_key = TextParameter::new("api_key", "API Key")
        .required()
        .sensitive()
        .min_length(10)
        .max_length(100)
        .placeholder("Enter your API key")
        .description("Your application API key from the dashboard");

    let timeout = NumberParameter::new("timeout", "Timeout")
        .default_value(30.0)
        .range(1.0, 300.0)
        .step(1.0)
        .precision(0)
        .description("Request timeout in seconds");

    let retries = NumberParameter::new("retries", "Retry Count")
        .default_value(3.0)
        .min(0.0)
        .max(10.0);

    // ── Example 2: Using macros for quick definitions ──────────────────────

    let enabled = param_def!(checkbox "enabled", "Enabled");
    let secret = param_def!(secret "token", "Auth Token");
    let port = param_def!(number "port", "Port", default = 8080.0);

    // ── Example 3: Building a collection ────────────────────────────────────

    let collection = ParameterCollection::new()
        .with(ParameterDef::Text(api_key))
        .with(ParameterDef::Number(timeout))
        .with(ParameterDef::Number(retries))
        .with(enabled)
        .with(secret)
        .with(port);

    println!("Created collection with {} parameters", collection.len());

    // ── Example 4: Creating values with macro ───────────────────────────────

    let values = param_values! {
        "api_key" => "secret123456789",
        "timeout" => 45,
        "retries" => 5,
        "enabled" => true,
        "token" => "bearer_token_xyz",
        "port" => 8080,
    };

    println!("Created values for {} parameters", values.len());

    // ── Example 5: Validation ───────────────────────────────────────────────

    match collection.validate(&values) {
        Ok(()) => println!("✓ All parameters valid"),
        Err(errors) => {
            println!("✗ Validation failed with {} errors:", errors.len());
            for error in errors {
                println!("  - [{}] {}", error.code(), error);
            }
        }
    }

    // ── Example 6: Type-safe value access ───────────────────────────────────

    if let Some(key) = values.get_string("api_key") {
        println!("API Key: {}", mask_string(key));
    }

    if let Some(timeout) = values.get_f64("timeout") {
        println!("Timeout: {}s", timeout);
    }

    if let Some(enabled) = values.get_bool("enabled") {
        println!("Enabled: {}", enabled);
    }

    // ── Example 7: Using ParameterType trait ────────────────────────────────

    let param = TextParameter::new("username", "Username")
        .required()
        .description("Your account username")
        .placeholder("Enter username")
        .min_length(3);

    // Trait methods are available
    println!("Parameter key: {}", param.key());
    println!("Is required: {}", param.is_required());
    println!("Is sensitive: {}", param.is_sensitive());

    // ── Example 8: Snapshot and restore ─────────────────────────────────────

    let snapshot = values.snapshot();
    let mut modified_values = values.clone();
    modified_values.set("timeout", 60.0.into());

    println!("Original timeout: {:?}", values.get_f64("timeout"));
    println!("Modified timeout: {:?}", modified_values.get_f64("timeout"));

    modified_values.restore(&snapshot);
    println!("Restored timeout: {:?}", modified_values.get_f64("timeout"));

    // ── Example 9: Diff tracking ────────────────────────────────────────────

    let new_values = param_values! {
        "api_key" => "secret123456789",
        "timeout" => 60,  // Changed
        "enabled" => true,
        "new_param" => "added",  // Added
        // "retries" removed
    };

    let diff = values.diff(&new_values);
    println!("\nChanges detected:");
    if !diff.added.is_empty() {
        println!("  Added: {:?}", diff.added);
    }
    if !diff.removed.is_empty() {
        println!("  Removed: {:?}", diff.removed);
    }
    if !diff.changed.is_empty() {
        println!("  Changed: {:?}", diff.changed);
    }
}

fn mask_string(s: &str) -> String {
    if s.len() <= 4 {
        "*".repeat(s.len())
    } else {
        format!("{}...{}", &s[..2], &s[s.len() - 2..])
    }
}
