//! ADR-0052 seam: validator is the sole `required` emitter, and a hidden
//! field that nonetheless carries a present value is STILL structurally
//! validated (a smuggled expression in a no-payload mode-variant placeholder
//! must not escape to resolve). The carve-out is moved, not deleted.
use nebula_schema::mode::VisibilityMode;
use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

#[test]
fn hidden_mode_present_expr_payload_is_rejected_not_skipped() {
    // A Mode field, hidden (Never). Variant `flag` is no-payload. An attacker
    // submits an expression payload to the hidden mode. Pre-fold the runner
    // reached validate_field via the raw.is_some() branch and rejected the
    // expression. Post-fold (validator sole emitter) the hidden+present field
    // MUST still be structurally validated → expression.forbidden.
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth"))
                .visible(VisibilityMode::Never)
                .variant_empty("flag", "Flag"),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "auth": { "mode": "flag", "value": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden+present mode payload must still be structurally validated, got: {:?}",
        report
            .errors()
            .map(|e| (e.code.to_string(), e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

fn codes(report: &nebula_schema::ValidationReport) -> Vec<(String, String)> {
    report
        .errors()
        .map(|e| (e.code.to_string(), e.path.to_string()))
        .collect()
}

#[test]
fn hidden_object_present_expr_child_is_still_validated() {
    // A hidden Object whose child forbids expressions. A submitted expression
    // in the nested child must still be rejected — the gate must recurse into
    // a hidden-but-present container, not skip it.
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("cfg"))
                .visible(VisibilityMode::Never)
                .add(Field::string(field_key!("token")).no_expression()),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "cfg": { "token": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden Object child must still be structurally validated, got: {:?}",
        codes(&report)
    );
}

#[test]
fn hidden_list_present_expr_item_is_still_validated() {
    // A hidden List whose item forbids expressions. A submitted expression in
    // an item must still be rejected (hidden-but-present list recursion).
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("rows"))
                .visible(VisibilityMode::Never)
                .item(Field::string(field_key!("v")).no_expression()),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "rows": [ { "$expr": "{{ $secrets.leak }}" } ]
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden List item must still be structurally validated, got: {:?}",
        codes(&report)
    );
}

#[test]
fn hidden_required_object_present_expr_child_is_validated_not_required_absent() {
    // Boundary pin: a hidden AND required Object that carries a present value.
    // `is_absent_for_required` has no Object arm, so the object counts as
    // present (value_present = true) → the directive must be Validate, NOT
    // RequiredAbsent. If a future Object arm is added to
    // `is_absent_for_required`, this would flip to RequiredAbsent, silently
    // stop recursing, and reopen the smuggled-expression fail-open — this test
    // is the tripwire.
    let schema = Schema::builder()
        .add(
            Field::object(field_key!("cfg"))
                .visible(VisibilityMode::Never)
                .required()
                .add(Field::string(field_key!("token")).no_expression()),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({
        "cfg": { "token": { "$expr": "{{ $secrets.leak }}" } }
    }))
    .unwrap();

    let report = schema.validate(&values).expect_err("must reject");
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "hidden+required Object with a present child must be Validated, not \
         swallowed by RequiredAbsent, got: {:?}",
        codes(&report)
    );
}

#[test]
fn hidden_required_empty_collection_emits_exactly_one_required() {
    // Double-emit tripwire beyond the flat-string case: a hidden + required
    // List supplied as an empty array is required-and-absent. Exactly one
    // `required` (validator is the sole emitter; the deleted gate-side builder
    // must not be reintroduced anywhere).
    let schema = Schema::builder()
        .add(
            Field::list(field_key!("rows"))
                .visible(VisibilityMode::Never)
                .required()
                .item(Field::string(field_key!("v"))),
        )
        .build()
        .expect("schema builds");

    let values = FieldValues::from_json(json!({ "rows": [] })).unwrap();
    let report = schema.validate(&values).expect_err("must reject");
    let required_count = report.errors().filter(|e| e.code == "required").count();
    assert_eq!(
        required_count,
        1,
        "exactly one `required` expected, got: {:?}",
        codes(&report)
    );
}
