//! Example demonstrating complex parameter types: Select, MultiSelect, Object.
//!
//! Run with: `cargo run --example typed_complex`

use nebula_parameter::option::SelectOption;
use nebula_parameter::typed::prelude::*;
use serde_json::json;

fn main() {
    println!("=== Typed Parameter API - Complex Types ===\n");

    // ── Select ───────────────────────────────────────────────────────────────

    println!("🔽 Select (single choice):");

    let region = Select::builder("region")
        .label("AWS Region")
        .description("Choose your deployment region")
        .option(SelectOption::new(
            "us-east-1",
            "US East (N. Virginia)",
            json!("us-east-1"),
        ))
        .option(SelectOption::new(
            "eu-west-1",
            "EU West (Ireland)",
            json!("eu-west-1"),
        ))
        .option(SelectOption::new(
            "ap-southeast-1",
            "Asia Pacific (Singapore)",
            json!("ap-southeast-1"),
        ))
        .default_value(json!("us-east-1"))
        .placeholder("Select a region...")
        .required()
        .build();

    println!("  Region select: {:?}", region.metadata.key);
    println!("  Options: {}", region.options.len());
    println!("  Default: {:?}", region.default);
    println!("  Required: {}", region.metadata.required);

    // ── MultiSelect ──────────────────────────────────────────────────────────

    println!("\n🔽 MultiSelect (multiple choice):");

    let tags = MultiSelect::builder("tags")
        .label("Environment Tags")
        .description("Select applicable tags")
        .option(SelectOption::new("prod", "Production", json!("prod")))
        .option(SelectOption::new("staging", "Staging", json!("staging")))
        .option(SelectOption::new("dev", "Development", json!("dev")))
        .option(SelectOption::new("test", "Testing", json!("test")))
        .min_selections(1)
        .max_selections(3)
        .default_values(vec![json!("dev")])
        .build();

    println!("  Tags multi-select: {:?}", tags.metadata.key);
    println!("  Options: {}", tags.options.len());
    println!(
        "  Min selections: {:?}",
        tags.multi_select_options.as_ref().unwrap().min_selections
    );
    println!(
        "  Max selections: {:?}",
        tags.multi_select_options.as_ref().unwrap().max_selections
    );

    // ── Object ───────────────────────────────────────────────────────────────

    println!("\n📦 Object (grouped parameters):");

    let db_config = Object::builder("database")
        .label("Database Configuration")
        .description("PostgreSQL connection settings")
        .field(
            Text::<Plain>::builder("host")
                .label("Host")
                .description("Database server hostname")
                .default_value("localhost")
                .required()
                .build()
                .into(),
        )
        .field(
            Number::<Port>::builder("port")
                .label("Port")
                .default_value(5432)
                .required()
                .build()
                .into(),
        )
        .field(
            Text::<Plain>::builder("database")
                .label("Database Name")
                .default_value("myapp")
                .required()
                .build()
                .into(),
        )
        .field(
            Text::<Plain>::builder("username")
                .label("Username")
                .required()
                .build()
                .into(),
        )
        .field(
            Text::<Password>::builder("password")
                .label("Password")
                .required()
                .build()
                .into(),
        )
        .collapsible(true)
        .required()
        .build();

    println!("  Database object: {:?}", db_config.metadata.key);
    println!("  Fields: {}", db_config.fields.len());
    println!(
        "  Collapsible: {}",
        db_config.options.as_ref().unwrap().collapsible
    );

    // Field details
    for field in &db_config.fields {
        println!("    - {}: {}", field.key(), field.name());
    }

    // ── Nested Object ────────────────────────────────────────────────────────

    println!("\n📦 Nested Object (object within object):");

    let api_config = Object::builder("api")
        .label("API Configuration")
        .field(
            Text::<Url>::builder("endpoint")
                .label("API Endpoint")
                .default_value("https://api.example.com")
                .required()
                .build()
                .into(),
        )
        .field(
            Object::builder("auth")
                .label("Authentication")
                .field(
                    Select::builder("method")
                        .label("Auth Method")
                        .option(SelectOption::new("bearer", "Bearer Token", json!("bearer")))
                        .option(SelectOption::new("basic", "Basic Auth", json!("basic")))
                        .option(SelectOption::new("apikey", "API Key", json!("apikey")))
                        .default_value(json!("bearer"))
                        .build()
                        .into(),
                )
                .field(
                    Text::<Password>::builder("token")
                        .label("Token / API Key")
                        .required()
                        .build()
                        .into(),
                )
                .collapsible(true)
                .build()
                .into(),
        )
        .field(
            Number::<GenericNumber>::builder("timeout")
                .label("Request Timeout (seconds)")
                .default_value(30.0)
                .build()
                .into(),
        )
        .build();

    println!("  API config: {:?}", api_config.metadata.key);
    println!("  Top-level fields: {}", api_config.fields.len());

    // ── Summary ──────────────────────────────────────────────────────────────

    println!("\n✅ Complex parameter types:");
    println!("  - Select: single-choice dropdowns with options");
    println!("  - MultiSelect: multi-choice with min/max constraints");
    println!("  - Object: grouped parameters with nesting support");
    println!("\n✅ All complex types have builder patterns and type safety");
    println!("✅ Objects can contain any parameter type, including other objects");
}
