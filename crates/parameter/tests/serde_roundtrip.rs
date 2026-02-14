use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::display::{DisplayCondition, DisplayRule, DisplayRuleSet, ParameterDisplay};
use nebula_parameter::kind::ParameterKind;
use nebula_parameter::option::SelectOption;
use nebula_parameter::types::*;
use nebula_parameter::validation::ValidationRule;
use nebula_parameter::values::ParameterValues;
use serde_json::json;

/// Build a ParameterCollection containing one instance of every 14 parameter types.
fn build_full_collection() -> ParameterCollection {
    let mut text = TextParameter::new("host", "Hostname");
    text.default = Some("localhost".into());
    text.metadata.required = true;
    text.options = Some(TextOptions {
        pattern: Some(r"^[a-zA-Z0-9.\-]+$".into()),
        max_length: Some(253),
        min_length: Some(1),
    });
    text.validation = vec![
        ValidationRule::min_length(1),
        ValidationRule::max_length(253),
    ];

    let mut textarea = TextareaParameter::new("notes", "Notes");
    textarea.default = Some("Enter notes here".into());
    textarea.options = Some(TextareaOptions {
        min_length: Some(10),
        max_length: Some(5000),
        rows: Some(8),
    });

    let mut code = CodeParameter::new("query", "SQL Query");
    code.default = Some("SELECT 1".into());
    code.options = Some(CodeOptions {
        language: CodeLanguage::Sql,
        line_numbers: true,
    });

    let secret = SecretParameter::new("api_key", "API Key");

    let mut number = NumberParameter::new("port", "Port");
    number.default = Some(8080.0);
    number.options = Some(NumberOptions {
        min: Some(1.0),
        max: Some(65535.0),
        step: Some(1.0),
        precision: Some(0),
    });
    number.validation = vec![ValidationRule::min(1.0), ValidationRule::max(65535.0)];

    let mut checkbox = CheckboxParameter::new("debug", "Debug Mode");
    checkbox.default = Some(false);
    checkbox.options = Some(CheckboxOptions {
        label: Some("Enable debug logging".into()),
        help_text: Some("Verbose output to stderr".into()),
    });

    let mut select = SelectParameter::new("region", "Region");
    select.default = Some(json!("us-east-1"));
    select.options = vec![
        SelectOption::new("us_east", "US East", json!("us-east-1")),
        SelectOption::new("eu_west", "EU West", json!("eu-west-1")),
    ];
    select.select_options = Some(SelectOptions {
        placeholder: Some("Choose a region...".into()),
    });

    let mut multi_select = MultiSelectParameter::new("features", "Features");
    multi_select.default = Some(vec![json!("logging")]);
    multi_select.options = vec![
        SelectOption::new("logging", "Logging", json!("logging")),
        SelectOption::new("metrics", "Metrics", json!("metrics")),
        SelectOption::new("tracing", "Tracing", json!("tracing")),
    ];
    multi_select.multi_select_options = Some(MultiSelectOptions {
        min_selections: Some(1),
        max_selections: Some(3),
    });

    let mut color = ColorParameter::new("accent", "Accent Color");
    color.default = Some("#ff5500".into());
    color.options = Some(ColorOptions {
        format: ColorFormat::Hex,
    });

    let mut datetime = DateTimeParameter::new("scheduled_at", "Scheduled At");
    datetime.default = Some("2026-03-01T10:00:00Z".into());
    datetime.options = Some(DateTimeOptions {
        min: Some("2020-01-01T00:00:00Z".into()),
        max: Some("2030-12-31T23:59:59Z".into()),
        format: Some("%Y-%m-%dT%H:%M:%S".into()),
    });

    let mut date = DateParameter::new("deadline", "Deadline");
    date.default = Some("2026-12-31".into());
    date.options = Some(DateOptions {
        min: Some("2026-01-01".into()),
        max: None,
        format: Some("%Y-%m-%d".into()),
    });

    let mut time = TimeParameter::new("start_time", "Start Time");
    time.default = Some("09:00".into());
    time.options = Some(TimeOptions {
        min: Some("08:00".into()),
        max: Some("18:00".into()),
        format: None,
        use_24h: true,
    });

    let mut hidden = HiddenParameter::new("node_version", "Node Version");
    hidden.default = Some(json!(2));

    let notice = NoticeParameter::new(
        "deprecation_notice",
        "Deprecation Notice",
        NoticeType::Warning,
        "This node will be removed in v3.",
    );

    ParameterCollection::new()
        .with(ParameterDef::Text(text))
        .with(ParameterDef::Textarea(textarea))
        .with(ParameterDef::Code(code))
        .with(ParameterDef::Secret(secret))
        .with(ParameterDef::Number(number))
        .with(ParameterDef::Checkbox(checkbox))
        .with(ParameterDef::Select(select))
        .with(ParameterDef::MultiSelect(multi_select))
        .with(ParameterDef::Color(color))
        .with(ParameterDef::DateTime(datetime))
        .with(ParameterDef::Date(date))
        .with(ParameterDef::Time(time))
        .with(ParameterDef::Hidden(hidden))
        .with(ParameterDef::Notice(notice))
}

// ---------------------------------------------------------------------------
// 1. Full collection round-trip
// ---------------------------------------------------------------------------

#[test]
fn collection_round_trips_all_14_types() {
    let original = build_full_collection();
    assert_eq!(original.len(), 14);

    let json_str = serde_json::to_string_pretty(&original).expect("serialization should not fail");
    let restored: ParameterCollection =
        serde_json::from_str(&json_str).expect("deserialization should not fail");

    assert_eq!(restored.len(), 14);

    // Verify keys are preserved in insertion order.
    let expected_keys = [
        "host",
        "notes",
        "query",
        "api_key",
        "port",
        "debug",
        "region",
        "features",
        "accent",
        "scheduled_at",
        "deadline",
        "start_time",
        "node_version",
        "deprecation_notice",
    ];
    let actual_keys: Vec<&str> = restored.keys().collect();
    assert_eq!(actual_keys, expected_keys);

    // Verify kinds round-trip.
    let expected_kinds = [
        ParameterKind::Text,
        ParameterKind::Textarea,
        ParameterKind::Code,
        ParameterKind::Secret,
        ParameterKind::Number,
        ParameterKind::Checkbox,
        ParameterKind::Select,
        ParameterKind::MultiSelect,
        ParameterKind::Color,
        ParameterKind::DateTime,
        ParameterKind::Date,
        ParameterKind::Time,
        ParameterKind::Hidden,
        ParameterKind::Notice,
    ];
    for (i, expected_kind) in expected_kinds.iter().enumerate() {
        let param = restored.get(i).unwrap();
        assert_eq!(param.kind(), *expected_kind, "kind mismatch at index {i}");
    }
}

#[test]
fn collection_round_trip_preserves_names() {
    let original = build_full_collection();
    let json_str = serde_json::to_string(&original).unwrap();
    let restored: ParameterCollection = serde_json::from_str(&json_str).unwrap();

    for (orig, rest) in original.iter().zip(restored.iter()) {
        assert_eq!(orig.name(), rest.name());
    }
}

#[test]
fn collection_round_trip_preserves_required() {
    let original = build_full_collection();
    let json_str = serde_json::to_string(&original).unwrap();
    let restored: ParameterCollection = serde_json::from_str(&json_str).unwrap();

    // "host" is the only required parameter in our collection.
    assert!(restored.get_by_key("host").unwrap().is_required());
    assert!(!restored.get_by_key("notes").unwrap().is_required());
}

#[test]
fn collection_round_trip_preserves_sensitive() {
    let original = build_full_collection();
    let json_str = serde_json::to_string(&original).unwrap();
    let restored: ParameterCollection = serde_json::from_str(&json_str).unwrap();

    assert!(restored.get_by_key("api_key").unwrap().is_sensitive());
    assert!(!restored.get_by_key("host").unwrap().is_sensitive());
}

// ---------------------------------------------------------------------------
// 2. Individual ParameterDef variant type-tag verification
// ---------------------------------------------------------------------------

#[test]
fn parameter_def_type_tags() {
    let cases: Vec<(ParameterDef, &str)> = vec![
        (ParameterDef::Text(TextParameter::new("a", "A")), "text"),
        (
            ParameterDef::Textarea(TextareaParameter::new("a", "A")),
            "textarea",
        ),
        (ParameterDef::Code(CodeParameter::new("a", "A")), "code"),
        (
            ParameterDef::Secret(SecretParameter::new("a", "A")),
            "secret",
        ),
        (
            ParameterDef::Number(NumberParameter::new("a", "A")),
            "number",
        ),
        (
            ParameterDef::Checkbox(CheckboxParameter::new("a", "A")),
            "checkbox",
        ),
        (
            ParameterDef::Select(SelectParameter::new("a", "A")),
            "select",
        ),
        (
            ParameterDef::MultiSelect(MultiSelectParameter::new("a", "A")),
            "multi_select",
        ),
        (ParameterDef::Color(ColorParameter::new("a", "A")), "color"),
        (
            ParameterDef::DateTime(DateTimeParameter::new("a", "A")),
            "date_time",
        ),
        (ParameterDef::Date(DateParameter::new("a", "A")), "date"),
        (ParameterDef::Time(TimeParameter::new("a", "A")), "time"),
        (
            ParameterDef::Hidden(HiddenParameter::new("a", "A")),
            "hidden",
        ),
        (
            ParameterDef::Notice(NoticeParameter::new("a", "A", NoticeType::Info, "msg")),
            "notice",
        ),
    ];

    for (def, expected_tag) in &cases {
        let json_str = serde_json::to_string(def).unwrap();
        let expected_fragment = format!("\"type\":\"{}\"", expected_tag);
        assert!(
            json_str.contains(&expected_fragment),
            "Expected type tag '{}' in JSON for {:?}, got: {}",
            expected_tag,
            def.key(),
            json_str
        );

        // Verify round-trip.
        let restored: ParameterDef = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.key(), def.key());
        assert_eq!(restored.kind(), def.kind());
    }
}

// ---------------------------------------------------------------------------
// 3. ParameterValues serialization (flat JSON)
// ---------------------------------------------------------------------------

#[test]
fn parameter_values_flat_json() {
    let mut vals = ParameterValues::new();
    vals.set("host", json!("localhost"));
    vals.set("port", json!(8080));
    vals.set("debug", json!(true));
    vals.set("tags", json!(["web", "api"]));

    let json_str = serde_json::to_string(&vals).unwrap();

    // Flat structure: no "values" wrapper key.
    assert!(!json_str.contains("\"values\""));

    let restored: ParameterValues = serde_json::from_str(&json_str).unwrap();
    assert_eq!(restored, vals);
    assert_eq!(restored.get_string("host"), Some("localhost"));
    assert_eq!(restored.get_f64("port"), Some(8080.0));
    assert_eq!(restored.get_bool("debug"), Some(true));
    assert_eq!(restored.get("tags"), Some(&json!(["web", "api"])));
}

#[test]
fn parameter_values_empty_round_trip() {
    let vals = ParameterValues::new();
    let json_str = serde_json::to_string(&vals).unwrap();
    assert_eq!(json_str, "{}");

    let restored: ParameterValues = serde_json::from_str(&json_str).unwrap();
    assert!(restored.is_empty());
}

// ---------------------------------------------------------------------------
// 4. ParameterDef with display conditions and validation rules
// ---------------------------------------------------------------------------

#[test]
fn parameter_with_display_conditions_round_trips() {
    let display = ParameterDisplay {
        show_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "mode".into(),
            condition: DisplayCondition::Equals {
                value: json!("advanced"),
            },
        })],
        hide_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "locked".into(),
            condition: DisplayCondition::IsTrue,
        })],
    };

    let mut text = TextParameter::new("advanced_option", "Advanced Option");
    text.display = Some(display);
    text.validation = vec![
        ValidationRule::min_length(1),
        ValidationRule::max_length(100),
        ValidationRule::pattern(r"^[a-z_]+$"),
    ];

    let def = ParameterDef::Text(text);
    let json_str = serde_json::to_string_pretty(&def).unwrap();
    let restored: ParameterDef = serde_json::from_str(&json_str).unwrap();

    // Display rules survived.
    let restored_display = restored.display().expect("display should be present");
    assert_eq!(restored_display.show_when.len(), 1);
    assert_eq!(restored_display.hide_when.len(), 1);

    // Validation rules survived.
    match &restored {
        ParameterDef::Text(p) => assert_eq!(p.validation.len(), 3),
        other => panic!("expected Text variant, got {:?}", other.kind()),
    }
}

#[test]
fn parameter_with_nested_display_conditions_round_trips() {
    let nested_display = ParameterDisplay {
        show_when: vec![DisplayRuleSet::All {
            rules: vec![
                DisplayRuleSet::Single(DisplayRule {
                    field: "enabled".into(),
                    condition: DisplayCondition::IsTrue,
                }),
                DisplayRuleSet::Any {
                    rules: vec![
                        DisplayRuleSet::Single(DisplayRule {
                            field: "mode".into(),
                            condition: DisplayCondition::Equals {
                                value: json!("pro"),
                            },
                        }),
                        DisplayRuleSet::Single(DisplayRule {
                            field: "mode".into(),
                            condition: DisplayCondition::Equals {
                                value: json!("enterprise"),
                            },
                        }),
                    ],
                },
            ],
        }],
        hide_when: vec![DisplayRuleSet::Not {
            rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                field: "visible".into(),
                condition: DisplayCondition::IsTrue,
            })),
        }],
    };

    let mut number = NumberParameter::new("concurrency", "Concurrency");
    number.display = Some(nested_display);
    number.default = Some(4.0);

    let def = ParameterDef::Number(number);
    let json_str = serde_json::to_string(&def).unwrap();
    let restored: ParameterDef = serde_json::from_str(&json_str).unwrap();

    let display = restored.display().expect("display should be present");
    assert_eq!(display.show_when.len(), 1);
    assert_eq!(display.hide_when.len(), 1);

    // Verify the All -> [Single, Any[Single, Single]] structure survived.
    match &display.show_when[0] {
        DisplayRuleSet::All { rules } => {
            assert_eq!(rules.len(), 2);
            match &rules[1] {
                DisplayRuleSet::Any { rules: any_rules } => {
                    assert_eq!(any_rules.len(), 2);
                }
                other => panic!("expected Any, got {:?}", other),
            }
        }
        other => panic!("expected All, got {:?}", other),
    }
}

#[test]
fn select_with_options_round_trips() {
    let mut select = SelectParameter::new("format", "Output Format");
    select.default = Some(json!("json"));
    select.options = vec![
        SelectOption::new("json", "JSON", json!("json")),
        SelectOption::new("xml", "XML", json!("xml")),
        SelectOption::new("csv", "CSV", json!("csv")),
    ];
    select.metadata.required = true;
    select.validation = vec![ValidationRule::OneOf {
        values: vec![json!("json"), json!("xml"), json!("csv")],
        message: Some("Must be json, xml, or csv".into()),
    }];

    let def = ParameterDef::Select(select);
    let json_str = serde_json::to_string(&def).unwrap();
    let restored: ParameterDef = serde_json::from_str(&json_str).unwrap();

    assert!(restored.is_required());
    match &restored {
        ParameterDef::Select(p) => {
            assert_eq!(p.options.len(), 3);
            assert_eq!(p.options[0].key, "json");
            assert_eq!(p.options[1].value, json!("xml"));
            assert_eq!(p.default, Some(json!("json")));
            assert_eq!(p.validation.len(), 1);
        }
        other => panic!("expected Select, got {:?}", other.kind()),
    }
}

#[test]
fn deserialize_from_raw_json_object() {
    let raw = json!({
        "type": "number",
        "key": "timeout",
        "name": "Request Timeout",
        "default": 30.0,
        "required": true,
        "options": {
            "min": 1.0,
            "max": 300.0,
            "step": 1.0
        },
        "validation": [
            { "rule": "min", "value": 1.0 },
            { "rule": "max", "value": 300.0 }
        ]
    });

    let def: ParameterDef = serde_json::from_value(raw).unwrap();
    assert_eq!(def.key(), "timeout");
    assert_eq!(def.name(), "Request Timeout");
    assert_eq!(def.kind(), ParameterKind::Number);
    assert!(def.is_required());

    match &def {
        ParameterDef::Number(p) => {
            assert_eq!(p.default, Some(30.0));
            assert_eq!(p.validation.len(), 2);
        }
        other => panic!("expected Number, got {:?}", other.kind()),
    }
}
