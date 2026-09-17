use nebula_validator::{Predicate, Rule};
use serde_json::json;

use super::*;
use crate::{FieldKey, error::ValidationReport, field::Property, field_key, path::FieldPath};

fn run(fields: &[Property]) -> ValidationReport {
    let mut report = ValidationReport::new();
    lint_tree(fields, &FieldPath::root(), &mut report);
    report
}

fn predicate_rule(predicate: Predicate) -> Rule {
    Rule::predicate(predicate).unwrap()
}

#[test]
fn detects_duplicate_key() {
    let fields = vec![
        Property::string(FieldKey::new("x").unwrap()).into_property(),
        Property::number(FieldKey::new("x").unwrap()).into_property(),
    ];
    let report = run(&fields);
    assert!(report.errors().any(|e| e.code() == "duplicate_key"));
}

#[test]
fn passes_clean_fields() {
    let fields = vec![
        Property::string(FieldKey::new("a").unwrap()).into_property(),
        Property::number(FieldKey::new("b").unwrap()).into_property(),
    ];
    let report = run(&fields);
    assert!(!report.has_errors());
}

#[test]
fn root_rule_rejects_unknown_field_reference() {
    let result = crate::Schema::builder()
        .property(Property::string(FieldKey::new("tier").unwrap()))
        .root_rule(predicate_rule(
            Predicate::eq("/missing", json!("pro")).unwrap(),
        ))
        .build();

    let report = result.expect_err("root rule should fail lint");
    assert!(
        report.errors().any(|e| e.code() == "dangling_reference"),
        "expected dangling_reference, got: {report:?}"
    );
}

#[test]
fn root_rule_accepts_nested_field_reference() {
    let result = crate::Schema::builder()
        .property(
            Property::object(FieldKey::new("config").unwrap())
                .property(Property::string(FieldKey::new("tier").unwrap())),
        )
        .root_rule(predicate_rule(
            Predicate::eq("/config/tier", json!("pro")).unwrap(),
        ))
        .build();

    assert!(result.is_ok(), "expected valid root rule, got: {result:?}");
}

#[test]
fn root_rule_accepts_list_object_child_reference() {
    let result = crate::Schema::builder()
        .property(
            Property::list(FieldKey::new("items").unwrap()).item(
                Property::object(FieldKey::new("row").unwrap())
                    .property(Property::string(FieldKey::new("name").unwrap())),
            ),
        )
        .root_rule(predicate_rule(
            Predicate::eq("/items/name", json!("x")).unwrap(),
        ))
        .build();

    assert!(result.is_ok(), "expected valid root rule, got: {result:?}");
}

#[test]
fn detects_missing_item_schema() {
    let fields = vec![Property::list(FieldKey::new("items").unwrap()).into_property()];
    let report = run(&fields);
    assert!(report.errors().any(|e| e.code() == "missing_item_schema"));
}

#[test]
fn detects_invalid_default_variant() {
    let fields = vec![
        Property::mode(FieldKey::new("m").unwrap())
            .default_variant("nonexistent")
            .into_property(),
    ];
    let report = run(&fields);
    assert!(
        report
            .errors()
            .any(|e| e.code() == "invalid_default_variant")
    );
}

#[test]
fn detects_duplicate_variant() {
    let fields = vec![
        Property::mode(FieldKey::new("m").unwrap())
            .variant("v1", "V1", Property::string(FieldKey::new("x").unwrap()))
            .variant(
                "v1",
                "V1 again",
                Property::string(FieldKey::new("y").unwrap()),
            )
            .into_property(),
    ];
    let report = run(&fields);
    assert!(report.errors().any(|e| e.code() == "duplicate_variant"));
}

#[test]
fn detects_invalid_mode_variant_key() {
    let fields = vec![
        Property::mode(FieldKey::new("m").unwrap())
            .variant(
                "oauth-token",
                "OAuth",
                Property::string(FieldKey::new("x").unwrap()),
            )
            .into_property(),
    ];
    let report = run(&fields);
    assert!(report.errors().any(|e| e.code() == "invalid_key"));
}

#[test]
fn detects_visibility_cycle_between_top_level_fields() {
    let fields = vec![
        Property::string(FieldKey::new("a").unwrap())
            .visible_when(predicate_rule(Predicate::eq("/b", json!("on")).unwrap()))
            .into_property(),
        Property::string(FieldKey::new("b").unwrap())
            .visible_when(predicate_rule(Predicate::eq("/a", json!("on")).unwrap()))
            .into_property(),
    ];
    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "visibility_cycle"),
        "expected visibility_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_visibility_cycle_inside_nested_object() {
    let outer = Property::object(field_key!("outer"))
        .property(
            Property::string(field_key!("x"))
                .visible_when(predicate_rule(
                    Predicate::eq("/outer/y", json!(true)).unwrap(),
                ))
                .into_property(),
        )
        .property(
            Property::string(field_key!("y"))
                .visible_when(predicate_rule(
                    Predicate::eq("/outer/x", json!(true)).unwrap(),
                ))
                .into_property(),
        );
    let report = run(&[outer.into()]);
    assert!(
        report.errors().any(|e| e.code() == "visibility_cycle"),
        "expected visibility_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn acyclic_visibility_rules_do_not_error() {
    let fields = vec![
        Property::string(field_key!("toggle")).into_property(),
        Property::string(field_key!("detail"))
            .visible_when(predicate_rule(
                Predicate::eq("/toggle", json!(true)).unwrap(),
            ))
            .into_property(),
    ];
    let report = run(&fields);
    assert!(!report.errors().any(|e| e.code() == "visibility_cycle"));
}

#[test]
fn detects_visibility_cycle_with_list_index_reference() {
    let fields = vec![
        Property::list(field_key!("items"))
            .item(
                Property::object(field_key!("row"))
                    .property(
                        Property::string(field_key!("x"))
                            .visible_when(predicate_rule(
                                Predicate::eq("/items/0/y", json!(true)).unwrap(),
                            ))
                            .into_property(),
                    )
                    .property(
                        Property::string(field_key!("y"))
                            .visible_when(predicate_rule(
                                Predicate::eq("/items/0/x", json!(true)).unwrap(),
                            ))
                            .into_property(),
                    ),
            )
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "visibility_cycle"),
        "expected visibility_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn pointer_refs_in_nested_scope_are_checked_against_root_keys() {
    let fields = vec![
        Property::object(field_key!("outer"))
            .property(
                Property::string(field_key!("x"))
                    .visible_when(predicate_rule(
                        Predicate::eq("/outer/y", json!(true)).unwrap(),
                    ))
                    .into_property(),
            )
            .into_property(),
        Property::string(field_key!("top")).into_property(),
    ];

    let report = run(&fields);
    assert!(
        !report.errors().any(|e| e.code() == "dangling_reference"),
        "did not expect dangling_reference, got {:?}",
        report
            .errors()
            .map(|e| (e.code(), e.path().to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_visibility_cycle_through_mode_variant_payload() {
    let fields = vec![
        Property::string(field_key!("a"))
            .visible_when(predicate_rule(Predicate::eq("/m/v", json!(true)).unwrap()))
            .into_property(),
        Property::mode(field_key!("m"))
            .variant(
                "v",
                "Variant",
                Property::string(field_key!("payload"))
                    .visible_when(predicate_rule(Predicate::eq("/a", json!(true)).unwrap()))
                    .into_property(),
            )
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "visibility_cycle"),
        "expected visibility_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_required_cycle_between_top_level_fields() {
    let fields = vec![
        Property::string(field_key!("a"))
            .required_when(predicate_rule(Predicate::eq("/b", json!(true)).unwrap()))
            .into_property(),
        Property::string(field_key!("b"))
            .required_when(predicate_rule(Predicate::eq("/a", json!(true)).unwrap()))
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "required_cycle"),
        "expected required_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_required_cycle_inside_nested_object() {
    let outer = Property::object(field_key!("outer"))
        .property(
            Property::string(field_key!("x"))
                .required_when(predicate_rule(
                    Predicate::eq("/outer/y", json!(true)).unwrap(),
                ))
                .into_property(),
        )
        .property(
            Property::string(field_key!("y"))
                .required_when(predicate_rule(
                    Predicate::eq("/outer/x", json!(true)).unwrap(),
                ))
                .into_property(),
        );

    let report = run(&[outer.into()]);
    assert!(
        report.errors().any(|e| e.code() == "required_cycle"),
        "expected required_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_required_cycle_with_list_index_reference() {
    let fields = vec![
        Property::list(field_key!("items"))
            .item(
                Property::object(field_key!("row"))
                    .property(
                        Property::string(field_key!("x"))
                            .required_when(predicate_rule(
                                Predicate::eq("/items/0/y", json!(true)).unwrap(),
                            ))
                            .into_property(),
                    )
                    .property(
                        Property::string(field_key!("y"))
                            .required_when(predicate_rule(
                                Predicate::eq("/items/0/x", json!(true)).unwrap(),
                            ))
                            .into_property(),
                    ),
            )
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "required_cycle"),
        "expected required_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_required_cycle_through_mode_variant_payload() {
    let fields = vec![
        Property::string(field_key!("a"))
            .required_when(predicate_rule(Predicate::eq("/m/v", json!(true)).unwrap()))
            .into_property(),
        Property::mode(field_key!("m"))
            .variant(
                "v",
                "Variant",
                Property::string(field_key!("payload"))
                    .required_when(predicate_rule(Predicate::eq("/a", json!(true)).unwrap()))
                    .into_property(),
            )
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "required_cycle"),
        "expected required_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn detects_visibility_and_required_cycles_independently() {
    let fields = vec![
        Property::string(field_key!("a"))
            .visible_when(predicate_rule(Predicate::eq("/b", json!(true)).unwrap()))
            .required_when(predicate_rule(Predicate::eq("/b", json!(true)).unwrap()))
            .into_property(),
        Property::string(field_key!("b"))
            .visible_when(predicate_rule(Predicate::eq("/a", json!(true)).unwrap()))
            .required_when(predicate_rule(Predicate::eq("/a", json!(true)).unwrap()))
            .into_property(),
    ];

    let report = run(&fields);
    assert!(
        report.errors().any(|e| e.code() == "visibility_cycle"),
        "expected visibility_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
    assert!(
        report.errors().any(|e| e.code() == "required_cycle"),
        "expected required_cycle, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn secret_field_with_default_emits_secret_default_forbidden() {
    use serde_json::json;
    let fields = vec![
        Property::secret(FieldKey::new("api_key").unwrap())
            .default(json!("hardcoded-token"))
            .into_property(),
    ];
    let report = run(&fields);
    assert!(
        report
            .errors()
            .any(|e| e.code() == "secret.default_forbidden"),
        "expected secret.default_forbidden, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn secret_field_without_default_passes_lint() {
    let fields = vec![
        Property::secret(FieldKey::new("api_key").unwrap())
            .required()
            .into_property(),
    ];
    let report = run(&fields);
    assert!(
        !report.has_errors(),
        "expected no errors, got {:?}",
        report
            .errors()
            .map(ValidationError::code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn legacy_root_rule_reference_is_a_hard_error_with_pointer_rewrite() {
    for field_ref in ["$root.tier", "/$root/tier"] {
        let mut report = ValidationReport::new();
        lint_legacy_root_reference(field_ref, &FieldPath::root(), &mut report);
        assert!(
            report.errors().any(|e| e.code() == "reference.legacy_root"),
            "expected legacy root reference error, got {:?}",
            report
                .iter()
                .map(|e| (e.code(), e.severity()))
                .collect::<Vec<_>>()
        );
        let error = report
            .errors()
            .find(|e| e.code() == "reference.legacy_root")
            .expect("error");
        assert_eq!(
            error.params()[1].1.as_str(),
            Some("/tier"),
            "suggested JSON Pointer"
        );
    }
}
