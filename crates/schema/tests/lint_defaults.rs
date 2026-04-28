//! Tests for the `default.type_mismatch` lint pass.

use nebula_schema::{Field, ValidSchema, ValidationReport, field_key};
use serde_json::{Value, json};

fn build(fields: impl IntoIterator<Item = Field>) -> Result<ValidSchema, ValidationReport> {
    let mut b = nebula_schema::Schema::builder();
    for f in fields {
        b = b.add(f);
    }
    b.build()
}

fn has_type_mismatch(fields: impl IntoIterator<Item = Field>) -> bool {
    match build(fields) {
        Ok(_) => false,
        Err(report) => report.errors().any(|e| e.code == "default.type_mismatch"),
    }
}

fn builds_ok(fields: impl IntoIterator<Item = Field>) -> bool {
    build(fields).is_ok()
}

// ── String ────────────────────────────────────────────────────────────────────

#[test]
fn default_string_valid() {
    let field = Field::string(field_key!("greeting"))
        .default(json!("hello"))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_string_invalid() {
    let field = Field::string(field_key!("greeting"))
        .default(json!(42))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── Number ────────────────────────────────────────────────────────────────────

#[test]
fn default_number_valid() {
    let field = Field::number(field_key!("ratio"))
        .default(json!(2.72))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_number_invalid() {
    let field = Field::number(field_key!("ratio"))
        .default(json!("not a number"))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── Integer ───────────────────────────────────────────────────────────────────

#[test]
fn default_integer_valid() {
    let field = Field::number(field_key!("count"))
        .integer()
        .default(json!(5))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_integer_invalid() {
    let field = Field::number(field_key!("count"))
        .integer()
        .default(json!(2.72))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── Boolean ───────────────────────────────────────────────────────────────────

#[test]
fn default_boolean_valid() {
    let field = Field::boolean(field_key!("enabled"))
        .default(json!(true))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_boolean_invalid() {
    let field = Field::boolean(field_key!("enabled"))
        .default(json!("true"))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── List ──────────────────────────────────────────────────────────────────────

#[test]
fn default_list_valid() {
    let field = Field::list(field_key!("tags"))
        .item(Field::string(field_key!("item")))
        .default(json!([1, 2, 3]))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_list_invalid() {
    let field = Field::list(field_key!("tags"))
        .item(Field::string(field_key!("item")))
        .default(json!("not a list"))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── Select ───────────────────────────────────────────────────────────────────

#[test]
fn default_select_multiple_array_valid() {
    let field = Field::select(field_key!("colors"))
        .option(json!("a"), "A")
        .option(json!("b"), "B")
        .option(json!("c"), "C")
        .multiple()
        .default(json!(["a", "b"]))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_select_multiple_array_invalid_element() {
    let field = Field::select(field_key!("colors"))
        .option(json!("a"), "A")
        .option(json!("b"), "B")
        .multiple()
        .default(json!(["a", "z"]))
        .into_field();
    assert!(has_type_mismatch([field]));
}

#[test]
fn default_select_allow_custom_valid() {
    let field = Field::select(field_key!("tag"))
        .option(json!("foo"), "Foo")
        .allow_custom()
        .default(json!("anything"))
        .into_field();
    assert!(builds_ok([field]));
}

// ── Mode ───────────────────────────────────────────────────────────────────────

#[test]
fn default_mode_valid() {
    let field = Field::mode(field_key!("auth"))
        .variant("token", "Token", Field::string(field_key!("token")))
        .default(json!({"mode": "token", "value": null}))
        .into_field();
    assert!(builds_ok([field]));
}

#[test]
fn default_mode_extra_keys_invalid() {
    let field = Field::mode(field_key!("auth"))
        .variant("token", "Token", Field::string(field_key!("token")))
        .default(json!({"mode": "x", "extra": 1}))
        .into_field();
    assert!(has_type_mismatch([field]));
}

#[test]
fn default_mode_missing_mode_key_invalid() {
    let field = Field::mode(field_key!("auth"))
        .variant("token", "Token", Field::string(field_key!("token")))
        .default(json!({"value": 1}))
        .into_field();
    assert!(has_type_mismatch([field]));
}

// ── Null is always valid ──────────────────────────────────────────────────────

#[test]
fn default_null_always_valid() {
    let fields: Vec<Field> = vec![
        Field::string(field_key!("s"))
            .default(Value::Null)
            .into_field(),
        Field::number(field_key!("n"))
            .default(Value::Null)
            .into_field(),
        Field::boolean(field_key!("b"))
            .default(Value::Null)
            .into_field(),
        Field::list(field_key!("l"))
            .item(Field::string(field_key!("i")))
            .default(Value::Null)
            .into_field(),
    ];
    assert!(builds_ok(fields));
}
