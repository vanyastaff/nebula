use super::{
    ArrayDefinition, NumberDefinition, Presentation, Property, Requirement, Schema, ValueDefinition,
};
use crate::{
    AuthoredValue, Field, FieldKey, Predicate, Rule, ValidSchema, ValidationReport, field_key,
};
use serde_json::{Number, Value, json};

fn string() -> ValueDefinition {
    ValueDefinition::String { rules: Vec::new() }
}

fn boolean() -> ValueDefinition {
    ValueDefinition::Boolean { rules: Vec::new() }
}

fn property(key: FieldKey, value: ValueDefinition, requirement: Requirement) -> Property {
    Property {
        key,
        value,
        requirement,
        presentation: Presentation::default(),
    }
}

fn endpoint_definition() -> ValueDefinition {
    ValueDefinition::Record {
        properties: vec![
            property(field_key!("endpoint"), string(), Requirement::Required),
            property(field_key!("enabled"), boolean(), Requirement::Required),
        ],
        rules: Vec::new(),
    }
}

fn diagnostics(report: &ValidationReport) -> Vec<(String, String)> {
    report
        .errors()
        .map(|error| (error.path().to_string(), error.code().to_owned()))
        .collect()
}

fn resolve(schema: &ValidSchema, data: Value) -> Result<Value, ValidationReport> {
    schema
        .validate(AuthoredValue::from_data(data).expect("bounded literal data"))?
        .resolve_data()
        .map(crate::ResolvedValues::into_json)
}

fn build_one(key: FieldKey, definition: ValueDefinition) -> super::LoweredSchema {
    Schema::builder()
        .property(property(key, definition, Requirement::Required))
        .build()
        .unwrap_or_else(|report| panic!("prototype schema: {report:?}"))
}

#[test]
fn incompatible_value_rules_fail_closed_recursively_for_every_definition_kind() {
    let fractional = Number::from_f64(1.5).expect("finite decimal");
    let cases = [
        ValueDefinition::String {
            rules: vec![Rule::min_items(1)],
        },
        ValueDefinition::Boolean {
            rules: vec![Rule::one_of([json!("private_boolean_operand")]).unwrap()],
        },
        ValueDefinition::Number(NumberDefinition {
            integer: false,
            minimum: Number::from(0),
            maximum: Number::from(10),
            rules: vec![Rule::min_length(1)],
        }),
        ValueDefinition::Number(NumberDefinition {
            integer: true,
            minimum: Number::from(0),
            maximum: Number::from(10),
            rules: vec![Rule::one_of([Value::Number(fractional)]).unwrap()],
        }),
        ValueDefinition::Record {
            properties: Vec::new(),
            rules: vec![Rule::max_items(1)],
        },
        ValueDefinition::Array(ArrayDefinition {
            item: Box::new(string()),
            min_items: None,
            max_items: None,
            unique: false,
            rules: vec![Rule::min_value(1)],
        }),
    ];
    for definition in cases {
        let report = Schema::builder()
            .property(property(
                field_key!("value"),
                definition,
                Requirement::Required,
            ))
            .build()
            .expect_err("incompatible value rule rejected");
        assert_eq!(
            diagnostics(&report),
            vec![("/value".into(), "property.rule.incompatible".into())]
        );
        assert!(!format!("{report:?}").contains("private_boolean_operand"));
    }

    let composite = Rule::described(
        Rule::all([
            Rule::one_of([json!(true)]).unwrap(),
            Rule::not(Rule::min_length(1)).unwrap(),
        ])
        .unwrap(),
        "private composite message",
    )
    .unwrap();
    let report = Schema::builder()
        .property(property(
            field_key!("outer"),
            ValueDefinition::Record {
                properties: vec![property(
                    field_key!("flag"),
                    ValueDefinition::Boolean {
                        rules: vec![composite],
                    },
                    Requirement::Required,
                )],
                rules: Vec::new(),
            },
            Requirement::Required,
        ))
        .build()
        .expect_err("nested composite mismatch rejected");
    assert_eq!(
        diagnostics(&report),
        vec![("/outer/flag".into(), "property.rule.incompatible".into())]
    );
    assert!(!format!("{report:?}").contains("private composite message"));

    for definition in [
        ValueDefinition::String {
            rules: vec![Rule::email()],
        },
        ValueDefinition::Boolean {
            rules: vec![Rule::one_of([json!(true), json!(false)]).unwrap()],
        },
        ValueDefinition::Number(NumberDefinition {
            integer: false,
            minimum: Number::from(0),
            maximum: Number::from(10),
            rules: vec![Rule::greater_than(0)],
        }),
        ValueDefinition::Record {
            properties: Vec::new(),
            rules: vec![Rule::one_of([json!({})]).unwrap()],
        },
        ValueDefinition::Array(ArrayDefinition {
            item: Box::new(string()),
            min_items: None,
            max_items: None,
            unique: false,
            rules: vec![Rule::max_items(2)],
        }),
    ] {
        Schema::builder()
            .property(property(
                field_key!("value"),
                definition,
                Requirement::Optional,
            ))
            .build()
            .unwrap_or_else(|report| panic!("compatible rule rejected: {report:?}"));
    }
}

#[test]
fn equal_definitions_are_reusable_at_distinct_keys_and_nested_paths() {
    let number = ValueDefinition::Number(NumberDefinition {
        integer: true,
        minimum: Number::from(1),
        maximum: Number::from(9),
        rules: vec![Rule::greater_than(1)],
    });
    let array = ValueDefinition::Array(ArrayDefinition {
        item: Box::new(number),
        min_items: Some(1),
        max_items: Some(2),
        unique: true,
        rules: vec![Rule::min_items(1)],
    });
    let record = ValueDefinition::Record {
        properties: vec![
            property(field_key!("name"), string(), Requirement::Required),
            property(field_key!("scores"), array.clone(), Requirement::Required),
        ],
        rules: Vec::new(),
    };
    let lowered = Schema::builder()
        .property(property(
            field_key!("primary"),
            record.clone(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("backup"),
            record,
            Requirement::Required,
        ))
        .property(property(
            field_key!("history"),
            array,
            Requirement::Required,
        ))
        .build()
        .unwrap_or_else(|report| panic!("reused definitions: {report:?}"));

    let valid = json!({
        "primary": {"name": "one", "scores": [2, 3]},
        "backup": {"name": "two", "scores": [4]},
        "history": [5]
    });
    assert_eq!(resolve(&lowered.schema, valid.clone()).unwrap(), valid);
    let invalid = json!({
        "primary": {"name": "one", "scores": [2]},
        "backup": {"name": "two", "scores": [1]},
        "history": [5]
    });
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, invalid).unwrap_err()),
        vec![("/backup/scores/0".into(), "greater_than".into())]
    );
}

#[test]
fn number_definition_preserves_mode_bounds_and_rules_canonically() {
    let definition = ValueDefinition::Number(NumberDefinition {
        integer: true,
        minimum: Number::from(-2),
        maximum: Number::from(8),
        rules: vec![Rule::greater_than(0), Rule::less_than(7)],
    });
    let semantic_json = serde_json::to_value(&definition).unwrap();
    assert_eq!(semantic_json["number"]["integer"], true);
    assert_eq!(semantic_json["number"]["minimum"], -2);
    assert_eq!(semantic_json["number"]["maximum"], 8);
    let lowered = build_one(field_key!("count"), definition);
    let Field::Number(field) = &lowered.schema.fields()[0] else {
        panic!("number field")
    };
    assert!(field.integer);
    assert_eq!(
        field.rules,
        vec![
            Rule::min_value(-2),
            Rule::max_value(8),
            Rule::greater_than(0),
            Rule::less_than(7),
        ]
    );
    for (value, code) in [(json!(-2), "greater_than"), (json!(7), "less_than")] {
        assert_eq!(
            diagnostics(&resolve(&lowered.schema, json!({"count": value})).unwrap_err()),
            vec![("/count".into(), code.into())]
        );
    }
    for (value, codes) in [
        (json!(-3), ["min", "greater_than"]),
        (json!(9), ["max", "less_than"]),
    ] {
        assert_eq!(
            diagnostics(&resolve(&lowered.schema, json!({"count": value})).unwrap_err()),
            codes
                .into_iter()
                .map(|code| ("/count".into(), code.into()))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({"count": 2.5})).unwrap_err()),
        vec![("/count".into(), "type_mismatch".into())]
    );
}

#[test]
fn number_mode_accepts_fractions_and_preserves_decimal_bounds_and_rules() {
    let minimum = Number::from_f64(-1.5).expect("finite minimum");
    let maximum = Number::from_f64(3.75).expect("finite maximum");
    let strict_minimum = Number::from_f64(-1.25).expect("finite strict minimum");
    let strict_rule = Rule::value(nebula_validator::ValueRule::GreaterThan(strict_minimum))
        .expect("bounded decimal rule");
    let definition = ValueDefinition::Number(NumberDefinition {
        integer: false,
        minimum: minimum.clone(),
        maximum: maximum.clone(),
        rules: vec![strict_rule.clone()],
    });
    let lowered = build_one(field_key!("ratio"), definition);
    let Field::Number(field) = &lowered.schema.fields()[0] else {
        panic!("number field")
    };
    assert!(!field.integer);
    assert_eq!(
        field.rules,
        vec![
            Rule::min_number(minimum),
            Rule::max_number(maximum),
            strict_rule,
        ]
    );
    assert_eq!(
        resolve(&lowered.schema, json!({"ratio": 1.5})).unwrap(),
        json!({"ratio": 1.5})
    );
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({"ratio": -1.4})).unwrap_err()),
        vec![("/ratio".into(), "greater_than".into())]
    );
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({"ratio": 4.0})).unwrap_err()),
        vec![("/ratio".into(), "max".into())]
    );
}

#[test]
fn array_constraints_and_item_rules_use_only_authored_index_paths() {
    let lowered = Schema::builder()
        .property(property(
            field_key!("tags"),
            ValueDefinition::Array(ArrayDefinition {
                item: Box::new(ValueDefinition::String {
                    rules: vec![Rule::min_length(2)],
                }),
                min_items: Some(1),
                max_items: Some(2),
                unique: true,
                rules: vec![Rule::max_items(2)],
            }),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("array schema: {report:?}"));
    let Field::List(field) = &lowered.schema.fields()[0] else {
        panic!("list field")
    };
    assert_eq!(field.min_items, Some(1));
    assert_eq!(field.max_items, Some(2));
    assert!(field.unique);
    assert_eq!(field.rules, vec![Rule::max_items(2)]);
    for (input, expected) in [
        (json!({"tags": []}), vec![("/tags", "items.min")]),
        (
            json!({"tags": ["ok", "ok"]}),
            vec![("/tags/1", "items.unique")],
        ),
        (json!({"tags": ["x"]}), vec![("/tags/0", "min_length")]),
    ] {
        let actual = diagnostics(&resolve(&lowered.schema, input).unwrap_err());
        assert_eq!(
            actual,
            expected
                .into_iter()
                .map(|(path, code)| (path.into(), code.into()))
                .collect::<Vec<_>>()
        );
        assert!(
            actual
                .iter()
                .all(|(path, _)| !path.contains("prototype_item"))
        );
    }
}

#[test]
fn context_free_composite_rules_survive_reconstruction() {
    let composite = Rule::all([
        Rule::min_length(2),
        Rule::any([
            Rule::one_of([json!("ab")]).unwrap(),
            Rule::not(Rule::one_of([json!("blocked")]).unwrap()).unwrap(),
        ])
        .unwrap(),
        Rule::described(Rule::max_length(5), "short value").unwrap(),
    ])
    .unwrap();
    let lowered = build_one(
        field_key!("name"),
        ValueDefinition::String {
            rules: vec![composite.clone()],
        },
    );
    assert_eq!(lowered.schema.fields()[0].rules(), &[composite]);
    assert_eq!(
        diagnostics(&resolve(&lowered.schema, json!({"name": "abcdef"})).unwrap_err()),
        vec![("/name".into(), "max_length".into())]
    );
}

#[test]
fn record_rules_are_preserved_and_run_against_the_record_value() {
    let allowed = json!({"endpoint": "ok", "enabled": true});
    let definition = ValueDefinition::Record {
        properties: vec![
            property(field_key!("endpoint"), string(), Requirement::Required),
            property(field_key!("enabled"), boolean(), Requirement::Required),
        ],
        rules: vec![Rule::one_of([allowed.clone()]).unwrap()],
    };
    let lowered = build_one(field_key!("connection"), definition);
    assert_eq!(
        resolve(&lowered.schema, json!({"connection": allowed})).unwrap(),
        json!({"connection": allowed})
    );
    assert_eq!(
        diagnostics(
            &resolve(
                &lowered.schema,
                json!({"connection": {"endpoint": "other", "enabled": true}})
            )
            .unwrap_err()
        ),
        vec![("/connection".into(), "one_of".into())]
    );
}

#[test]
fn contextual_and_deferred_rules_are_rejected_recursively_without_payloads() {
    let contextual = Rule::all([
        Rule::min_length(1),
        Rule::described(
            Rule::not(
                Rule::predicate(Predicate::eq("private_switch", json!("private_value")).unwrap())
                    .unwrap(),
            )
            .unwrap(),
            "private message",
        )
        .unwrap(),
    ])
    .unwrap();
    let deferred = Rule::any([
        Rule::min_items(1),
        Rule::custom("private_external_expression").unwrap(),
    ])
    .unwrap();
    for (definition, path, code, forbidden) in [
        (
            ValueDefinition::Record {
                properties: vec![property(
                    field_key!("name"),
                    ValueDefinition::String {
                        rules: vec![contextual],
                    },
                    Requirement::Required,
                )],
                rules: Vec::new(),
            },
            "/outer/name",
            "property.rule.contextual",
            "private_switch",
        ),
        (
            ValueDefinition::Array(ArrayDefinition {
                item: Box::new(string()),
                min_items: None,
                max_items: None,
                unique: false,
                rules: vec![deferred],
            }),
            "/outer",
            "property.rule.deferred",
            "private_external_expression",
        ),
    ] {
        let report = Schema::builder()
            .property(property(
                field_key!("outer"),
                definition,
                Requirement::Required,
            ))
            .build()
            .expect_err("contextual definition rejected");
        assert_eq!(diagnostics(&report), vec![(path.into(), code.into())]);
        assert!(!format!("{report:?}").contains(forbidden));
    }
}

#[test]
fn anonymous_array_item_rules_reject_context_recursively_without_adapter_paths() {
    let contextual = Rule::all([
        Rule::min_length(1),
        Rule::described(
            Rule::predicate(Predicate::eq("private_switch", json!(true)).unwrap()).unwrap(),
            "private contextual message",
        )
        .unwrap(),
    ])
    .unwrap();
    let deferred = Rule::not(
        Rule::described(
            Rule::custom("private deferred expression").unwrap(),
            "private deferred message",
        )
        .unwrap(),
    )
    .unwrap();
    for (item, path, code, private_payload) in [
        (
            ValueDefinition::String {
                rules: vec![contextual],
            },
            "/items",
            "property.rule.contextual",
            "private_switch",
        ),
        (
            ValueDefinition::Record {
                properties: vec![property(
                    field_key!("count"),
                    ValueDefinition::Number(NumberDefinition {
                        integer: true,
                        minimum: Number::from(0),
                        maximum: Number::from(10),
                        rules: vec![deferred],
                    }),
                    Requirement::Required,
                )],
                rules: Vec::new(),
            },
            "/items/count",
            "property.rule.deferred",
            "private deferred expression",
        ),
    ] {
        let report = Schema::builder()
            .property(property(
                field_key!("items"),
                ValueDefinition::Array(ArrayDefinition {
                    item: Box::new(item),
                    min_items: None,
                    max_items: None,
                    unique: false,
                    rules: Vec::new(),
                }),
                Requirement::Optional,
            ))
            .build()
            .expect_err("anonymous item context rejected");
        let actual = diagnostics(&report);
        assert_eq!(actual, vec![(path.into(), code.into())]);
        assert!(
            actual
                .iter()
                .all(|(diagnostic_path, _)| !diagnostic_path.contains("prototype_item"))
        );
        assert!(!format!("{report:?}").contains(private_payload));
    }
}

#[test]
fn generated_expressions_are_forbidden_at_property_record_and_array_levels() {
    let lowered = build_one(
        field_key!("record"),
        ValueDefinition::Record {
            properties: vec![property(
                field_key!("items"),
                ValueDefinition::Array(ArrayDefinition {
                    item: Box::new(endpoint_definition()),
                    min_items: None,
                    max_items: None,
                    unique: false,
                    rules: Vec::new(),
                }),
                Requirement::Required,
            )],
            rules: Vec::new(),
        },
    );
    for (data, path) in [
        (json!({"record": {"$expr": "1"}}), "/record"),
        (
            json!({"record": {"items": {"$expr": "1"}}}),
            "/record/items",
        ),
        (
            json!({"record": {"items": [{"$expr": "1"}]}}),
            "/record/items/0",
        ),
    ] {
        let report = lowered
            .schema
            .validate(AuthoredValue::from_template_json(data).unwrap())
            .unwrap_err();
        assert_eq!(
            diagnostics(&report),
            vec![(path.into(), "expression.forbidden".into())]
        );
    }
}

#[test]
fn cosmetic_annotations_serialize_separately_from_equal_semantics() {
    let build = |label: &str, hidden: bool| {
        let mut nested = property(field_key!("endpoint"), string(), Requirement::Required);
        nested.presentation.label = Some(label.into());
        nested.presentation.hidden = hidden;
        let definition = ValueDefinition::Record {
            properties: vec![nested],
            rules: Vec::new(),
        };
        let semantic_json = serde_json::to_value(&definition).unwrap();
        let lowered = build_one(field_key!("connection"), definition.clone());
        (definition, semantic_json, lowered)
    };
    let (shown_definition, shown_json, shown) = build("Endpoint", false);
    let (hidden_definition, hidden_json, hidden) = build("Renamed", true);
    assert_eq!(shown_definition, hidden_definition);
    assert_eq!(shown_json, hidden_json);
    assert_eq!(shown.schema, hidden.schema);
    assert_ne!(
        serde_json::to_value(&shown.annotations).unwrap(),
        serde_json::to_value(&hidden.annotations).unwrap()
    );
    for data in [
        json!({"connection": {"endpoint": "ok"}}),
        json!({"connection": {"endpoint": 2}}),
    ] {
        let outcome = |schema| resolve(schema, data.clone()).map_err(|report| diagnostics(&report));
        assert_eq!(outcome(&shown.schema), outcome(&hidden.schema));
    }
}

#[test]
fn semantic_changes_alter_definition_serialization_and_behavior() {
    let short = ValueDefinition::String {
        rules: vec![Rule::min_length(2)],
    };
    let long = ValueDefinition::String {
        rules: vec![Rule::min_length(3)],
    };
    assert_ne!(short, long);
    assert_ne!(
        serde_json::to_value(&short).unwrap(),
        serde_json::to_value(&long).unwrap()
    );
    assert_eq!(
        resolve(
            &build_one(field_key!("value"), short).schema,
            json!({"value": "ab"})
        )
        .unwrap(),
        json!({"value": "ab"})
    );
    assert_eq!(
        diagnostics(
            &resolve(
                &build_one(field_key!("value"), long).schema,
                json!({"value": "ab"})
            )
            .unwrap_err()
        ),
        vec![("/value".into(), "min_length".into())]
    );
}

#[test]
fn union_is_explicitly_rejected_at_its_occurrence_path() {
    let report = Schema::builder()
        .property(property(
            field_key!("choice"),
            ValueDefinition::Union,
            Requirement::Required,
        ))
        .build()
        .expect_err("union definition rejected");
    assert_eq!(
        diagnostics(&report),
        vec![("/choice".into(), "property.union".into())]
    );
}

#[test]
fn duplicate_keys_and_invalid_runtime_keys_use_checked_diagnostics() {
    let report = Schema::builder()
        .property(property(
            field_key!("value"),
            string(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("value"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .expect_err("duplicate property rejected");
    assert_eq!(
        diagnostics(&report),
        vec![("/value".into(), "duplicate_key".into())]
    );
    assert_eq!(FieldKey::new("bad-key").unwrap_err().code(), "invalid_key");
}

#[test]
fn legacy_requiredness_and_distinct_proof_origins_are_retained() {
    let optional = Schema::builder()
        .property(property(
            field_key!("value"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .unwrap();
    assert_eq!(resolve(&optional.schema, json!({})).unwrap(), json!({}));
    assert_eq!(
        diagnostics(&resolve(&optional.schema, json!({"value": null})).unwrap_err()),
        vec![("/value".into(), "type_mismatch".into())]
    );
    let first = build_one(field_key!("value"), string()).schema;
    let second = build_one(field_key!("value"), string()).schema;
    assert_eq!(first, second);
    assert!(!first.ptr_eq(&second));
    for (origin, other) in [(&first, &second), (&second, &first)] {
        let report = resolve(origin, json!({"value": ""})).unwrap_err();
        assert_eq!(
            diagnostics(&report),
            vec![("/value".into(), "required".into())]
        );
        let valid = origin
            .validate(AuthoredValue::from_data(json!({"value": "ok"})).unwrap())
            .unwrap();
        assert!(valid.schema().ptr_eq(origin));
        assert!(!valid.schema().ptr_eq(other));
        let resolved = valid.resolve_data().unwrap();
        assert!(resolved.schema().ptr_eq(origin));
        assert!(!resolved.schema().ptr_eq(other));
    }
}

#[test]
fn legacy_requiredness_characterizes_hidden_null_string_and_array_emptiness() {
    let mut hidden = property(field_key!("hidden"), string(), Requirement::Required);
    hidden.presentation.hidden = true;
    let strings = Schema::builder()
        .property(hidden)
        .property(property(
            field_key!("optional"),
            string(),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("string requiredness schema: {report:?}"));
    assert_eq!(
        diagnostics(&resolve(&strings.schema, json!({})).unwrap_err()),
        vec![("/hidden".into(), "required".into())]
    );
    assert_eq!(
        resolve(&strings.schema, json!({"hidden": "ok", "optional": ""})).unwrap(),
        json!({"hidden": "ok", "optional": ""})
    );
    assert_eq!(
        diagnostics(
            &resolve(&strings.schema, json!({"hidden": null, "optional": "ok"})).unwrap_err()
        ),
        vec![("/hidden".into(), "required".into())]
    );

    let array = || {
        ValueDefinition::Array(ArrayDefinition {
            item: Box::new(string()),
            min_items: None,
            max_items: None,
            unique: false,
            rules: Vec::new(),
        })
    };
    let arrays = Schema::builder()
        .property(property(
            field_key!("required"),
            array(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("optional"),
            array(),
            Requirement::Optional,
        ))
        .build()
        .unwrap_or_else(|report| panic!("array requiredness schema: {report:?}"));
    assert_eq!(
        diagnostics(&resolve(&arrays.schema, json!({"required": [], "optional": []})).unwrap_err()),
        vec![("/required".into(), "required".into())]
    );
    assert_eq!(
        resolve(&arrays.schema, json!({"required": ["ok"], "optional": []})).unwrap(),
        json!({"required": ["ok"], "optional": []})
    );
}

#[test]
fn annotation_paths_are_occurrences_not_definition_identity() {
    let mut definition = endpoint_definition();
    let ValueDefinition::Record { properties, .. } = &mut definition else {
        panic!("record definition")
    };
    let endpoint_presentation = Presentation {
        label: Some("Endpoint".into()),
        description: Some("Connection endpoint".into()),
        hidden: true,
    };
    properties[0].presentation = endpoint_presentation.clone();
    let lowered = Schema::builder()
        .property(property(
            field_key!("primary"),
            definition.clone(),
            Requirement::Required,
        ))
        .property(property(
            field_key!("backup"),
            definition,
            Requirement::Required,
        ))
        .build()
        .unwrap();
    let presented_occurrences = lowered
        .annotations
        .iter()
        .filter(|annotation| annotation.presentation != Presentation::default())
        .map(|annotation| (annotation.path.to_string(), annotation.presentation.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        presented_occurrences,
        vec![
            ("/primary/endpoint".into(), endpoint_presentation.clone()),
            ("/backup/endpoint".into(), endpoint_presentation),
        ]
    );
}
