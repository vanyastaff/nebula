//! Tests for the `default.type_mismatch` lint pass.

use nebula_schema::{Property, ValidSchema, ValidationReport, field_key};
use serde_json::{Value, json};

fn build(fields: impl IntoIterator<Item = Property>) -> Result<ValidSchema, ValidationReport> {
    let mut b = nebula_schema::Schema::builder();
    for f in fields {
        b = b.property(f);
    }
    b.build()
}

fn has_type_mismatch(fields: impl IntoIterator<Item = Property>) -> bool {
    match build(fields) {
        Ok(_) => false,
        Err(report) => report.errors().any(|e| e.code() == "default.type_mismatch"),
    }
}

fn builds_ok(fields: impl IntoIterator<Item = Property>) -> bool {
    build(fields).is_ok()
}

// ── String ────────────────────────────────────────────────────────────────────

#[test]
fn default_string_valid() {
    let field = Property::string(field_key!("greeting"))
        .default(json!("hello"))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_string_invalid() {
    let field = Property::string(field_key!("greeting"))
        .default(json!(42))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── Number ────────────────────────────────────────────────────────────────────

#[test]
fn default_number_valid() {
    let field = Property::number(field_key!("ratio"))
        .default(json!(2.72))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_number_invalid() {
    let field = Property::number(field_key!("ratio"))
        .default(json!("not a number"))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── Integer ───────────────────────────────────────────────────────────────────

#[test]
fn default_integer_valid() {
    let field = Property::number(field_key!("count"))
        .integer()
        .default(json!(5))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_integer_invalid() {
    let field = Property::number(field_key!("count"))
        .integer()
        .default(json!(2.72))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── Boolean ───────────────────────────────────────────────────────────────────

#[test]
fn default_boolean_valid() {
    let field = Property::boolean(field_key!("enabled"))
        .default(json!(true))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_boolean_invalid() {
    let field = Property::boolean(field_key!("enabled"))
        .default(json!("true"))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── List ──────────────────────────────────────────────────────────────────────

#[test]
fn default_list_valid() {
    let field = Property::list(field_key!("tags"))
        .item(Property::string(field_key!("item")))
        .default(json!([1, 2, 3]))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_list_invalid() {
    let field = Property::list(field_key!("tags"))
        .item(Property::string(field_key!("item")))
        .default(json!("not a list"))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── Select ───────────────────────────────────────────────────────────────────

#[test]
fn default_select_multiple_array_valid() {
    let field = Property::select(field_key!("colors"))
        .option(json!("a"), "A")
        .option(json!("b"), "B")
        .option(json!("c"), "C")
        .multiple()
        .default(json!(["a", "b"]))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_select_multiple_array_invalid_element() {
    let field = Property::select(field_key!("colors"))
        .option(json!("a"), "A")
        .option(json!("b"), "B")
        .multiple()
        .default(json!(["a", "z"]))
        .into_property();
    assert!(has_type_mismatch([field]));
}

#[test]
fn default_select_allow_custom_valid() {
    let field = Property::select(field_key!("tag"))
        .option(json!("foo"), "Foo")
        .allow_custom()
        .default(json!("anything"))
        .into_property();
    assert!(builds_ok([field]));
}

// ── Mode ───────────────────────────────────────────────────────────────────────

#[test]
fn default_mode_valid() {
    let field = Property::mode(field_key!("auth"))
        .variant("token", "Token", Property::string(field_key!("token")))
        .default(json!({"mode": "token", "value": null}))
        .into_property();
    assert!(builds_ok([field]));
}

#[test]
fn default_mode_extra_keys_invalid() {
    let field = Property::mode(field_key!("auth"))
        .variant("token", "Token", Property::string(field_key!("token")))
        .default(json!({"mode": "x", "extra": 1}))
        .into_property();
    assert!(has_type_mismatch([field]));
}

#[test]
fn default_mode_missing_mode_key_invalid() {
    let field = Property::mode(field_key!("auth"))
        .variant("token", "Token", Property::string(field_key!("token")))
        .default(json!({"value": 1}))
        .into_property();
    assert!(has_type_mismatch([field]));
}

// ── Null is always valid ──────────────────────────────────────────────────────

#[test]
fn default_null_always_valid() {
    let fields: Vec<Property> = vec![
        Property::string(field_key!("s"))
            .default(Value::Null)
            .into_property(),
        Property::number(field_key!("n"))
            .default(Value::Null)
            .into_property(),
        Property::boolean(field_key!("b"))
            .default(Value::Null)
            .into_property(),
        Property::list(field_key!("l"))
            .item(Property::string(field_key!("i")))
            .default(Value::Null)
            .into_property(),
    ];
    assert!(builds_ok(fields));
}
