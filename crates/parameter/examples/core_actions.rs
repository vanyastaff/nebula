//! Core control-flow actions — parameter schema example.
//!
//! Demonstrates mode parameters (discriminated unions), nested objects, lists,
//! and `required_when` / `visible_when` conditions for workflow control nodes:
//! `If`, `Switch`, `ForEach`, and `Wait`.
//!
//! Run with: `cargo run --example core_actions -p nebula-parameter`

use nebula_parameter::{Parameter, ParameterCollection, Rule};
use serde_json::json;

// ── If action ───────────────────────────────────────────────────────────────

/// `If` — evaluates a predicate and routes to `true` or `false` branch.
fn if_schema() -> ParameterCollection {
    ParameterCollection::new().add(
        // Condition is a mode: expression (raw string) or a structured builder
        Parameter::mode("condition")
            .label("Condition")
            .required()
            .variant(
                Parameter::string("expr")
                    .label("Expression")
                    .description("Evaluate a raw expression string")
                    .placeholder("{{ $input.value > 0 }}")
                    .required(),
            )
            .variant(
                Parameter::object("compare")
                    .label("Compare Values")
                    .description("Compare two values with an operator")
                    .add(Parameter::string("left").label("Left Value").required())
                    .add(
                        Parameter::select("operator")
                            .label("Operator")
                            .required()
                            .default(json!("eq"))
                            .option(json!("eq"), "equals (=)")
                            .option(json!("ne"), "not equals (\u{2260})")
                            .option(json!("gt"), "greater than (>)")
                            .option(json!("lt"), "less than (<)")
                            .option(json!("gte"), "\u{2265}")
                            .option(json!("lte"), "\u{2264}")
                            .option(json!("contains"), "contains")
                            .option(json!("matches"), "matches regex"),
                    )
                    .add(Parameter::string("right").label("Right Value").required()),
            )
            .default_variant("expr"),
    )
}

// ── Switch action ────────────────────────────────────────────────────────────

/// `Switch` — routes to one of N named branches based on a value.
fn switch_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(
            Parameter::string("value")
                .label("Value to Switch On")
                .description(
                    "Expression or field reference whose value determines the route",
                )
                .required(),
        )
        .add(
            Parameter::list(
                "cases",
                Parameter::object("case")
                    .add(
                        Parameter::string("match_value")
                            .label("Matches")
                            .placeholder("exact value or expression")
                            .required(),
                    )
                    .add(
                        Parameter::string("output")
                            .label("Output Branch Name")
                            .required()
                            .default(json!("branch_1")),
                    ),
            )
            .label("Cases")
            .description("Ordered list of match cases")
            .min_items(1),
        )
        .add(
            Parameter::string("fallback_output")
                .label("Fallback Branch")
                .description("Branch name used when no case matches")
                .default(json!("default")),
        )
}

// ── ForEach action ───────────────────────────────────────────────────────────

/// `ForEach` — iterates over a list, executing inner nodes per item.
fn for_each_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(
            Parameter::string("items")
                .label("Items")
                .description("Expression resolving to an array")
                .placeholder("{{ $input.rows }}")
                .required(),
        )
        .add(
            Parameter::string("item_variable")
                .label("Item Variable Name")
                .description("Name of the variable bound to each item in the loop body")
                .default(json!("item"))
                .with_rule(Rule::Pattern {
                    pattern: r"^[a-zA-Z_][a-zA-Z0-9_]*$".to_owned(),
                    message: Some("must be a valid identifier".to_owned()),
                }),
        )
        .add(
            Parameter::integer("batch_size")
                .label("Batch Size")
                .description("Number of items to process concurrently (1 = sequential)")
                .default(json!(1))
                .with_rule(Rule::Min {
                    min: serde_json::Number::from(1u64),
                    message: None,
                })
                .with_rule(Rule::Max {
                    max: serde_json::Number::from(100u64),
                    message: None,
                }),
        )
}

// ── Wait action ──────────────────────────────────────────────────────────────

/// `Wait` — pauses execution for a fixed duration or until a timestamp.
fn wait_schema() -> ParameterCollection {
    ParameterCollection::new().add(
        Parameter::mode("wait_mode")
            .label("Wait Until")
            .required()
            .variant(
                Parameter::object("duration")
                    .label("Fixed Duration")
                    .add(
                        Parameter::integer("amount")
                            .label("Amount")
                            .default(json!(1))
                            .required(),
                    )
                    .add(
                        Parameter::select("unit")
                            .label("Unit")
                            .default(json!("seconds"))
                            .option(json!("milliseconds"), "Milliseconds")
                            .option(json!("seconds"), "Seconds")
                            .option(json!("minutes"), "Minutes")
                            .option(json!("hours"), "Hours")
                            .option(json!("days"), "Days"),
                    ),
            )
            .variant(
                Parameter::string("timestamp")
                    .label("Until Timestamp")
                    .description("ISO 8601 datetime or expression")
                    .required(),
            )
            .default_variant("duration"),
    )
}

fn main() {
    let schemas: &[(&str, ParameterCollection)] = &[
        ("If", if_schema()),
        ("Switch", switch_schema()),
        ("ForEach", for_each_schema()),
        ("Wait", wait_schema()),
    ];

    for (name, schema) in schemas {
        println!("{name} ({} fields):", schema.len());
        for param in &schema.parameters {
            println!("  - {}", param.id);
        }

        let json = serde_json::to_string_pretty(schema).expect("serializes");
        println!("{json}\n");
    }
}
