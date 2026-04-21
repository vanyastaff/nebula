//! Schema-time validation tests for `ValidSchema::validate`.
//!
//! Covers: empty schema, required-field checks, type-mismatch,
//! expression-forbidden, expression-deferred, predicate rules via RuleContext.

use nebula_schema::*;
use serde_json::json;

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

// ── Empty schema ─────────────────────────────────────────────────────────────

#[test]
fn empty_schema_empty_values_ok() {
    let schema = Schema::builder().build().unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn empty_schema_with_extra_values_ok() {
    // Schema doesn't care about extra keys.
    let schema = Schema::builder().build().unwrap();
    let values = FieldValues::from_json(json!({"x": 1})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Required-field checks ────────────────────────────────────────────────────

#[test]
fn required_field_missing_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "required"),
        "expected required error"
    );
}

#[test]
fn required_field_null_value_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": null})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_field_present_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hello"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn optional_field_absent_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Expression handling ──────────────────────────────────────────────────────

#[test]
fn expression_in_allowed_field_deferred_not_error() {
    // ExpressionMode::Allowed (default for string) — expression skips value rules.
    let schema = Schema::builder()
        .add(Field::string(fk("x")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "{{ $ctx.value }}"})).unwrap();
    // Required field has expression value — must NOT produce "required" error.
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn expression_in_forbidden_mode_field_emits_error() {
    // BooleanField defaults to ExpressionMode::Forbidden.
    let schema = Schema::builder()
        .add(Field::boolean(fk("flag")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"flag": "{{ $x }}"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn expression_in_explicit_forbidden_string_emits_error() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")).no_expression())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "{{ $y }}"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "expression.forbidden"));
}

// ── Type mismatch ────────────────────────────────────────────────────────────

#[test]
fn string_field_number_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": 42})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

#[test]
fn number_field_string_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::number(fk("n")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"n": "not a number"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

#[test]
fn boolean_field_string_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(Field::boolean(fk("ok")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"ok": "yes"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "type_mismatch"));
}

// ── Rule evaluation (via RuleContext) ─────────────────────────────────────────

#[test]
fn length_max_rule_violated() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).max_length(5))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "toolongvalue"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "length.max"),
        "expected length.max error, codes: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn length_max_rule_satisfied() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).max_length(10))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "alice"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── ValidValues accessors ─────────────────────────────────────────────────────

#[test]
fn valid_values_exposes_warnings_empty_by_default() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let valid = schema.validate(&values).unwrap();
    assert!(valid.warnings().is_empty());
}

#[test]
fn valid_values_raw_values_matches_input() {
    let schema = Schema::builder()
        .add(Field::string(fk("x")))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"x": "hi"})).unwrap();
    let valid = schema.validate(&values).unwrap();
    let fk_x = FieldKey::new("x").unwrap();
    assert_eq!(
        valid.raw_values().get(&fk_x),
        Some(&FieldValue::Literal(json!("hi")))
    );
}

// ── Nested object validation ───────────────────────────────────────────────────

#[test]
fn nested_required_field_missing_emits_required() {
    let schema = Schema::builder()
        .add(Field::object(fk("user")).add(Field::string(fk("email")).required()))
        .build()
        .unwrap();
    // Provide user object but without email.
    let values = FieldValues::from_json(json!({"user": {}})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn nested_required_field_present_ok() {
    let schema = Schema::builder()
        .add(Field::object(fk("user")).add(Field::string(fk("email")).required()))
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"user": {"email": "a@b.com"}})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

// ── Select multiple/scalar mismatch (exhaustive check) ──────────────────────

#[test]
fn multi_select_with_scalar_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": "a"})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "type_mismatch"),
        "expected type_mismatch for scalar on multi select, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

#[test]
fn single_select_with_array_value_emits_type_mismatch() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("choice"))
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"choice": ["a", "b"]})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "type_mismatch"),
        "expected type_mismatch for array on single select, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

// ── Required + empty values ─────────────────────────────────────────────────

#[test]
fn required_string_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_secret_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::secret(fk("token")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"token": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_list_empty_emits_required() {
    let schema = Schema::builder()
        .add(
            Field::list(fk("items"))
                .item(Field::string(fk("it")))
                .required(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"items": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_multi_file_empty_array_emits_required() {
    let schema = Schema::builder()
        .add(Field::file(fk("uploads")).multiple().required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"uploads": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_code_empty_emits_required() {
    let schema = Schema::builder()
        .add(Field::code(fk("script")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"script": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_single_file_empty_string_emits_required() {
    let schema = Schema::builder()
        .add(Field::file(fk("upload")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"upload": ""})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn required_multi_select_empty_array_emits_required() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .required(),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": []})).unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn multi_select_with_expression_item_forbidden_emits_expression_forbidden() {
    // Select defaults to ExpressionMode::Forbidden. A multi-select value
    // whose list contains an expression placeholder must be rejected at
    // validate-time — otherwise `resolve` would silently evaluate it.
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({
        "tags": ["a", {"$expr": "{{ $dynamic }}"}]
    }))
    .unwrap();
    let report = schema.validate(&values).unwrap_err();
    assert!(
        report.errors().any(|e| e.code == "expression.forbidden"),
        "expected expression.forbidden, got: {:?}",
        report.errors().map(|e| &e.code).collect::<Vec<_>>()
    );
}

#[test]
fn required_string_single_char_ok() {
    let schema = Schema::builder()
        .add(Field::string(fk("name")).required())
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"name": "a"})).unwrap();
    assert!(schema.validate(&values).is_ok());
}

#[test]
fn multi_select_with_array_of_valid_options_ok() {
    let schema = Schema::builder()
        .add(
            Field::select(fk("tags"))
                .multiple()
                .option("a", "A")
                .option("b", "B"),
        )
        .build()
        .unwrap();
    let values = FieldValues::from_json(json!({"tags": ["a", "b"]})).unwrap();
    assert!(schema.validate(&values).is_ok());
}
