//! Core control-flow actions — parameter schema example.
//!
//! Demonstrates `Mode` fields (discriminated unions), `Predicate` fields,
//! and `required_when` / `visible_when` conditions for workflow control nodes:
//! `If`, `Switch`, `ForEach`, and `Wait`.
//!
//! Run with: `cargo run --example core_actions -p nebula-parameter`

use nebula_parameter::{
    Condition, Field, FieldMetadata, ModeVariant, OptionSource, Rule, Schema, SelectOption,
};
use serde_json::json;

// ── If action ───────────────────────────────────────────────────────────────

/// `If` — evaluates a predicate and routes to `true` or `false` branch.
fn if_schema() -> Schema {
    Schema::new()
        .field(
            // Condition is a mode: expression (raw string) or a structured builder
            Field::Mode {
                meta: {
                    let mut m = FieldMetadata::new("condition");
                    m.set_label("Condition");
                    m.required = true;
                    m
                },
                variants: vec![
                    ModeVariant {
                        key: "expression".to_owned(),
                        label: "Expression".to_owned(),
                        description: Some("Evaluate a raw expression string".to_owned()),
                        content: Box::new(
                            Field::text("expr")
                                .with_label("Expression")
                                .with_placeholder("{{ $input.value > 0 }}")
                                .required(),
                        ),
                    },
                    ModeVariant {
                        key: "compare".to_owned(),
                        label: "Compare Values".to_owned(),
                        description: Some("Compare two values with an operator".to_owned()),
                        content: Box::new(Field::Object {
                            meta: FieldMetadata::new("compare"),
                            fields: vec![
                                Field::text("left").with_label("Left Value").required(),
                                Field::Select {
                                    meta: {
                                        let mut m = FieldMetadata::new("operator");
                                        m.set_label("Operator");
                                        m.required = true;
                                        m.default = Some(json!("eq"));
                                        m
                                    },
                                    source: OptionSource::Static {
                                        options: vec![
                                            SelectOption::new(json!("eq"), "equals (=)"),
                                            SelectOption::new(json!("ne"), "not equals (≠)"),
                                            SelectOption::new(json!("gt"), "greater than (>)"),
                                            SelectOption::new(json!("lt"), "less than (<)"),
                                            SelectOption::new(json!("gte"), "≥"),
                                            SelectOption::new(json!("lte"), "≤"),
                                            SelectOption::new(json!("contains"), "contains"),
                                            SelectOption::new(json!("matches"), "matches regex"),
                                        ],
                                    },
                                    multiple: false,
                                    allow_custom: false,
                                    searchable: false,
                                },
                                Field::text("right").with_label("Right Value").required(),
                            ],
                        }),
                    },
                ],
                default_variant: Some("expression".to_owned()),
            },
        )
}

// ── Switch action ────────────────────────────────────────────────────────────

/// `Switch` — routes to one of N named branches based on a value.
fn switch_schema() -> Schema {
    Schema::new()
        .field(
            Field::text("value")
                .with_label("Value to Switch On")
                .with_description("Expression or field reference whose value determines the route")
                .required(),
        )
        .field(Field::List {
            meta: {
                let mut m = FieldMetadata::new("cases");
                m.set_label("Cases");
                m.set_description("Ordered list of match cases");
                m
            },
            item: Box::new(Field::Object {
                meta: FieldMetadata::new("case"),
                fields: vec![
                    Field::text("match_value")
                        .with_label("Matches")
                        .with_placeholder("exact value or expression")
                        .required(),
                    Field::text("output")
                        .with_label("Output Branch Name")
                        .required()
                        .with_default(json!("branch_1")),
                ],
            }),
            min_items: Some(1),
            max_items: None,
        })
        .field(
            Field::text("fallback_output")
                .with_label("Fallback Branch")
                .with_description("Branch name used when no case matches")
                .with_default(json!("default")),
        )
}

// ── ForEach action ───────────────────────────────────────────────────────────

/// `ForEach` — iterates over a list, executing inner nodes per item.
fn for_each_schema() -> Schema {
    Schema::new()
        .field(
            Field::text("items")
                .with_label("Items")
                .with_description("Expression resolving to an array")
                .with_placeholder("{{ $input.rows }}")
                .required(),
        )
        .field(
            Field::text("item_variable")
                .with_label("Item Variable Name")
                .with_description("Name of the variable bound to each item in the loop body")
                .with_default(json!("item"))
                .with_rule(Rule::Pattern {
                    pattern: r"^[a-zA-Z_][a-zA-Z0-9_]*$".to_owned(),
                    message: Some("must be a valid identifier".to_owned()),
                }),
        )
        .field(
            Field::integer("batch_size")
                .with_label("Batch Size")
                .with_description("Number of items to process concurrently (1 = sequential)")
                .with_default(json!(1))
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
fn wait_schema() -> Schema {
    Schema::new()
        .field(Field::Mode {
            meta: {
                let mut m = FieldMetadata::new("wait_mode");
                m.set_label("Wait Until");
                m.required = true;
                m
            },
            variants: vec![
                ModeVariant {
                    key: "duration".to_owned(),
                    label: "Fixed Duration".to_owned(),
                    description: None,
                    content: Box::new(Field::Object {
                        meta: FieldMetadata::new("duration"),
                        fields: vec![
                            Field::integer("amount")
                                .with_label("Amount")
                                .with_default(json!(1))
                                .required(),
                            Field::Select {
                                meta: {
                                    let mut m = FieldMetadata::new("unit");
                                    m.set_label("Unit");
                                    m.default = Some(json!("seconds"));
                                    m
                                },
                                source: OptionSource::Static {
                                    options: vec![
                                        SelectOption::new(json!("milliseconds"), "Milliseconds"),
                                        SelectOption::new(json!("seconds"), "Seconds"),
                                        SelectOption::new(json!("minutes"), "Minutes"),
                                        SelectOption::new(json!("hours"), "Hours"),
                                        SelectOption::new(json!("days"), "Days"),
                                    ],
                                },
                                multiple: false,
                                allow_custom: false,
                                searchable: false,
                            },
                        ],
                    }),
                },
                ModeVariant {
                    key: "timestamp".to_owned(),
                    label: "Until Timestamp".to_owned(),
                    description: None,
                    content: Box::new(
                        Field::text("timestamp")
                            .with_label("Timestamp")
                            .with_description("ISO 8601 datetime or expression")
                            .required(),
                    ),
                },
            ],
            default_variant: Some("duration".to_owned()),
        })
}

fn main() {
    let schemas: &[(&str, Schema)] = &[
        ("If", if_schema()),
        ("Switch", switch_schema()),
        ("ForEach", for_each_schema()),
        ("Wait", wait_schema()),
    ];

    for (name, schema) in schemas {
        println!("{name} ({} fields):", schema.fields.len());
        for field in &schema.fields {
            println!("  - {}", field.meta().id);
        }

        let json = serde_json::to_string_pretty(schema).expect("serializes");
        println!("{json}\n");
    }
}
