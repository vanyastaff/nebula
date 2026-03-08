//! Example demonstrating UI-oriented parameter types: Group, List, Notice.
//!
//! Run with: `cargo run --example typed_ui`

use nebula_parameter::typed::prelude::*;

fn main() {
    println!("=== Typed Parameter API - UI-Oriented Types ===\n");

    // ── Group ────────────────────────────────────────────────────────────────

    println!("📁 Group (visual grouping):");

    let advanced = Group::builder("advanced_settings")
        .label("Advanced Settings")
        .description("Additional configuration options")
        .parameter(
            Text::<Plain>::builder("custom_header")
                .label("Custom Header")
                .build()
                .into(),
        )
        .parameter(
            Number::<GenericNumber>::builder("timeout")
                .label("Timeout (seconds)")
                .default_value(30.0)
                .build()
                .into(),
        )
        .parameter(
            Checkbox::<Toggle>::builder("verbose_logging")
                .label("Enable Verbose Logging")
                .build()
                .into(),
        )
        .collapsible(true)
        .collapsed_by_default(true)
        .bordered(true)
        .build();

    println!("  Group: {:?}", advanced.metadata.key);
    println!("  Parameters in group: {}", advanced.parameters.len());
    println!(
        "  Collapsible: {}",
        advanced.options.as_ref().unwrap().collapsible
    );
    println!(
        "  Bordered: {}",
        advanced.options.as_ref().unwrap().bordered
    );

    // ── List ─────────────────────────────────────────────────────────────────

    println!("\n📋 List (repeatable items):");

    let env_vars = List::builder(
        "environment_variables",
        Object::builder("env_var")
            .field(
                Text::<Plain>::builder("name")
                    .label("Variable Name")
                    .required()
                    .build()
                    .into(),
            )
            .field(
                Text::<Plain>::builder("value")
                    .label("Variable Value")
                    .required()
                    .build()
                    .into(),
            )
            .build()
            .into(),
    )
    .label("Environment Variables")
    .description("Define environment variables for your application")
    .min_items(0)
    .max_items(20)
    .add_button_label("Add Variable")
    .sortable(true)
    .build();

    println!("  List: {:?}", env_vars.metadata.key);
    println!(
        "  Min items: {:?}",
        env_vars.options.as_ref().unwrap().min_items
    );
    println!(
        "  Max items: {:?}",
        env_vars.options.as_ref().unwrap().max_items
    );
    println!(
        "  Sortable: {}",
        env_vars.options.as_ref().unwrap().sortable
    );

    // ── List with simple items ───────────────────────────────────────────────

    println!("\n📋 List (simple text items):");

    let tags = List::builder(
        "tags",
        Text::<Plain>::builder("tag")
            .label("Tag")
            .required()
            .build()
            .into(),
    )
    .label("Tags")
    .description("Add tags to categorize this item")
    .min_items(1)
    .max_items(10)
    .add_button_label("Add Tag")
    .build();

    println!("  Tags list: {:?}", tags.metadata.key);
    println!("  Item template: {}", tags.item_template.key());

    // ── Notice ───────────────────────────────────────────────────────────────

    println!("\n📢 Notice (informational messages):");

    let info = Notice::info("api_info", "API Information")
        .content("This API uses OAuth 2.0 for authentication")
        .build();

    let warning = Notice::warning("deprecation", "Deprecation Warning")
        .content("This feature will be removed in version 3.0")
        .description("Plan your migration accordingly")
        .build();

    let error = Notice::error("config_error", "Configuration Error")
        .content("Invalid API key format. Must be 32 characters.")
        .build();

    let success = Notice::success("setup_complete", "Setup Complete")
        .content("Your configuration has been saved successfully!")
        .build();

    println!(
        "  Info notice: {:?} - {:?}",
        info.metadata.key, info.notice_type
    );
    println!("  Content: {}", info.content);

    println!(
        "\n  Warning notice: {:?} - {:?}",
        warning.metadata.key, warning.notice_type
    );
    println!("  Content: {}", warning.content);

    println!(
        "\n  Error notice: {:?} - {:?}",
        error.metadata.key, error.notice_type
    );
    println!("  Content: {}", error.content);

    println!(
        "\n  Success notice: {:?} - {:?}",
        success.metadata.key, success.notice_type
    );
    println!("  Content: {}", success.content);

    // ── Combined example ─────────────────────────────────────────────────────

    println!("\n🎨 Combined: Group with List and Notice:");

    let api_config = Group::builder("api_configuration")
        .label("API Configuration")
        .parameter(
            Notice::info("api_help", "Configuration Help")
                .content("Configure your API endpoints and authentication below")
                .build()
                .into(),
        )
        .parameter(
            Text::<Url>::builder("base_url")
                .label("Base URL")
                .default_value("https://api.example.com")
                .required()
                .build()
                .into(),
        )
        .parameter(
            List::builder(
                "headers",
                Object::builder("header")
                    .field(
                        Text::<Plain>::builder("key")
                            .label("Header Name")
                            .required()
                            .build()
                            .into(),
                    )
                    .field(
                        Text::<Plain>::builder("value")
                            .label("Header Value")
                            .required()
                            .build()
                            .into(),
                    )
                    .build()
                    .into(),
            )
            .label("Custom Headers")
            .min_items(0)
            .max_items(10)
            .add_button_label("Add Header")
            .build()
            .into(),
        )
        .parameter(
            Notice::warning("rate_limits", "Rate Limits")
                .content("This API has a rate limit of 1000 requests per hour")
                .build()
                .into(),
        )
        .collapsible(true)
        .bordered(true)
        .build();

    println!("  API config group: {:?}", api_config.metadata.key);
    println!("  Total items: {}", api_config.parameters.len());
    println!("  Items:");
    for param in &api_config.parameters {
        println!("    - {} ({})", param.name(), param.key());
    }

    // ── Summary ──────────────────────────────────────────────────────────────

    println!("\n✅ UI-oriented parameter types:");
    println!("  - Group: visual grouping (collapsible, bordered)");
    println!("  - List: repeatable items (min/max, sortable, custom template)");
    println!("  - Notice: informational messages (info/warning/error/success)");
    println!("\n✅ All types compose together for rich UI structures");
}
