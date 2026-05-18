//! (P2) lockdown #2: security-relevant codes are invariant.
//!
//! Dropping the validator→schema code translation changes the *rule-failure*
//! vocabulary (a deliberate, canon-legal breaking change). It must NOT change
//! the schema/policy-emitted codes that gate security-relevant behavior:
//! `required` (the validator is the sole emitter, but the wire code is
//! pinned), `expression.forbidden` (a literal/expression confusion guard),
//! and `expression.required` (an expression-only field rejecting a literal).
//! These flow through the single crossing unchanged.

use nebula_schema::{Field, FieldValues, Schema, field_key};
use serde_json::json;

fn codes(r: &nebula_schema::ValidationReport) -> Vec<String> {
    r.errors().map(|e| e.code.to_string()).collect()
}

#[test]
fn required_code_is_invariant_through_single_crossing() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("x")).required())
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    let got = codes(&err);
    assert!(
        got.iter().any(|c| c == "required"),
        "required must stay exactly `required`, got: {got:?}"
    );
}

#[test]
fn expression_forbidden_code_is_invariant() {
    // BooleanField has ExpressionMode::Forbidden by default; a `{{ }}`
    // literal must be rejected with the exact `expression.forbidden` code.
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("flag")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"flag": "{{ $x }}"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    let got = codes(&err);
    assert!(
        got.iter().any(|c| c == "expression.forbidden"),
        "expression.forbidden must stay exact, got: {got:?}"
    );
}

#[test]
fn expression_required_code_is_invariant() {
    // ComputedField is ExpressionMode::Required; a literal must be rejected
    // with the exact `expression.required` code.
    let schema = Schema::builder()
        .add(Field::computed(field_key!("derived")))
        .build()
        .unwrap();
    let vs = FieldValues::from_json(json!({"derived": "literal"})).unwrap();
    let err = schema.validate(&vs).unwrap_err();
    let got = codes(&err);
    assert!(
        got.iter().any(|c| c == "expression.required"),
        "expression.required must stay exact, got: {got:?}"
    );
}
