//! Example demonstrating the subtype system with semantic parameter types.
//!
//! Subtypes enable:
//! - Better UI widget selection (e.g., color picker for Color, slider for Percentage)
//! - Automatic validation pattern application
//! - Semantic intent expression
//! - Type-safe serialization with serde

use nebula_parameter::prelude::*;

fn main() {
    // ─── Text Subtypes ──────────────────────────────────────────────────────

    // Email parameter with automatic email validation
    let email = TextParameter::email("user_email", "User Email")
        .description("Contact email address")
        .required();

    println!("Email parameter:");
    println!("  Subtype: {:?}", email.subtype);
    println!("  Description: {}", email.subtype.description());
    println!("  Auto-validation: {} rules", email.validation.len());
    println!();

    // URL parameter with automatic URL validation
    let api_url = TextParameter::url("api_url", "API URL")
        .description("Base URL for API requests")
        .placeholder("https://api.example.com");

    println!("URL parameter:");
    println!("  Subtype: {:?}", api_url.subtype);
    println!("  Pattern: {:?}", api_url.subtype.validation_pattern());
    println!();

    // Password parameter (automatically marked as sensitive)
    let password = TextParameter::password("password", "Password")
        .min_length(8)
        .hint("Must be at least 8 characters");

    println!("Password parameter:");
    println!("  Subtype: {:?}", password.subtype);
    println!("  Is sensitive: {}", password.subtype.is_sensitive());
    println!("  Metadata.sensitive: {}", password.metadata.sensitive);
    println!();

    // Code parameters
    let json_config = TextParameter::new("config", "Configuration")
        .subtype(TextSubtype::Json)
        .description("JSON configuration object");

    let python_script = TextParameter::new("script", "Python Script")
        .subtype(TextSubtype::Python)
        .description("Python code to execute");

    println!("Code parameters:");
    println!("  JSON - is_code: {}", json_config.subtype.is_code());
    println!("  Python - is_code: {}", python_script.subtype.is_code());
    println!();

    // ─── Number Subtypes ────────────────────────────────────────────────────

    // Port number with automatic range constraints
    let port = NumberParameter::port("server_port", "Server Port")
        .description("HTTP server port")
        .default_value(8080.0);

    println!("Port parameter:");
    println!("  Subtype: {:?}", port.subtype);
    println!("  Min: {:?}", port.options.as_ref().and_then(|o| o.min));
    println!("  Max: {:?}", port.options.as_ref().and_then(|o| o.max));
    println!();

    // Percentage with 0-100 range
    let opacity =
        NumberParameter::percentage("opacity", "Opacity").description("Element opacity level");

    println!("Percentage parameter:");
    println!("  Subtype: {:?}", opacity.subtype);
    println!("  Is percentage: {}", opacity.subtype.is_percentage());
    println!(
        "  Range: {:?}-{:?}",
        opacity.options.as_ref().and_then(|o| o.min),
        opacity.options.as_ref().and_then(|o| o.max)
    );
    println!();

    // Custom subtypes with specific constraints
    let temperature = NumberParameter::new("temp", "Temperature")
        .subtype(NumberSubtype::Temperature)
        .description("Ambient temperature")
        .range(-50.0, 100.0)
        .precision(1);

    let distance = NumberParameter::new("radius", "Radius")
        .subtype(NumberSubtype::Distance)
        .min(0.0)
        .max(1000.0)
        .step(0.1);

    println!("Physical units:");
    println!("  Temperature: {}", temperature.subtype.description());
    println!("  Distance: {}", distance.subtype.description());
    println!();

    // ─── Serde Integration ──────────────────────────────────────────────────

    // Subtypes serialize to snake_case strings
    let params = ParameterCollection::new()
        .with(ParameterDef::Text(email.clone()))
        .with(ParameterDef::Number(port.clone()));

    let json = serde_json::to_string_pretty(&params).unwrap();
    println!("Serialized collection:\n{}\n", json);

    // Deserialize back
    let deserialized: ParameterCollection = serde_json::from_str(&json).unwrap();
    println!(
        "Successfully deserialized collection with {} parameters",
        serde_json::to_value(&deserialized).unwrap()["parameters"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0)
    );
    println!();

    // ─── All Text Subtypes ──────────────────────────────────────────────────

    println!("All Text Subtypes:");
    let text_subtypes: Vec<TextSubtype> = vec![
        TextSubtype::Plain,
        TextSubtype::Email,
        TextSubtype::Url,
        TextSubtype::FilePath,
        TextSubtype::ColorHex,
        TextSubtype::Password,
        TextSubtype::Json,
        TextSubtype::Yaml,
        TextSubtype::Xml,
        TextSubtype::JavaScript,
        TextSubtype::Python,
        TextSubtype::Rust,
        TextSubtype::Sql,
        TextSubtype::Regex,
        TextSubtype::Uuid,
        TextSubtype::Semver,
        TextSubtype::Markdown,
        TextSubtype::Html,
        TextSubtype::Css,
    ];

    for subtype in &text_subtypes {
        println!("  {:?}: {}", subtype, subtype.description());
    }
    println!();

    // ─── All Number Subtypes ────────────────────────────────────────────────

    println!("All Number Subtypes:");
    let number_subtypes: Vec<NumberSubtype> = vec![
        NumberSubtype::Distance,
        NumberSubtype::Angle,
        NumberSubtype::Time,
        NumberSubtype::Temperature,
        NumberSubtype::Speed,
        NumberSubtype::Mass,
        NumberSubtype::Volume,
        NumberSubtype::Energy,
        NumberSubtype::Percentage,
        NumberSubtype::Port,
        NumberSubtype::ByteSize,
        NumberSubtype::Currency,
        NumberSubtype::Timestamp,
    ];

    for subtype in &number_subtypes {
        println!("  {:?}: {}", subtype, subtype.description());
    }
}
