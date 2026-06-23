//! Schema-evolution gate (C13): golden snapshots that **freeze the serde wire
//! representation** of the schema-definition types.
//!
//! `Field` is `#[serde(tag = "type")]` and is re-exported as a real external
//! contract (`nebula-api`'s public-schema projection consumes it), so a renamed
//! variant, a renamed field, or a changed type is a **silent** wire break that
//! no compiler catches. Any such change diffs a snapshot below and fails CI
//! until a maintainer consciously accepts it (`cargo insta review`).
//!
//! Backward/forward-compatibility rule this gate enforces by review:
//! - new `Field` variants are only safe because the enum is `#[non_exhaustive]`
//!   (an old reader must not be required to match them exhaustively);
//! - new struct fields must be `Option` / `#[serde(default)]` with
//!   `skip_serializing_if`, so a document written by an older version still
//!   deserializes and a document written by a newer version still round-trips;
//! - a `type` an old reader does not recognize deserializes to `Field::Unknown`
//!   and is preserved verbatim (see `unknown_field_type_preserved_verbatim`), so
//!   a newer writer's field kind never fails to read on an older deployment.

use nebula_schema::{
    Field, FieldValue, FieldValues, Predicate, Rule, Schema, VisibilityMode, field_key,
};
use serde_json::json;

/// Every `Field` variant's wire shape (the `type` tag + each struct's
/// non-skipped fields). A renamed variant / field / type tag diffs here.
#[test]
fn field_variants_wire_format() {
    let variants: Vec<Field> = vec![
        // A fully-decorated field freezes the SHARED serde keys that are skipped
        // when default (`label`/`description`/`placeholder`/`default`/`group`/
        // `visible`/`rules`), so renaming any of them also diffs this snapshot.
        Field::string(field_key!("s"))
            .label("Display Name")
            .description("A fully-described field")
            .placeholder("type here")
            .group("contact")
            .default(json!("default-value"))
            .visible(VisibilityMode::Never)
            .with_rule(Rule::predicate(Predicate::eq("s", json!("x")).unwrap()))
            .required()
            .into(),
        Field::secret(field_key!("sec")).into(),
        Field::number(field_key!("n")).integer().into(),
        Field::boolean(field_key!("b")).into(),
        Field::select(field_key!("sel")).option("a", "A").into(),
        Field::object(field_key!("o"))
            .add(Field::string(field_key!("inner")))
            .into(),
        Field::list(field_key!("l"))
            .item(Field::string(field_key!("it")))
            .into(),
        Field::mode(field_key!("m"))
            .variant("v", "V", Field::string(field_key!("x")))
            .into(),
        Field::code(field_key!("c")).into(),
        Field::file(field_key!("f")).multiple().into(),
        Field::computed(field_key!("comp")).into(),
        Field::dynamic(field_key!("d")).into(),
        Field::notice(field_key!("not")).into(),
    ];
    insta::assert_json_snapshot!(variants);
}

/// A built `ValidSchema`'s wire shape (the `{"fields": [...]}` envelope plus a
/// representative field set).
#[test]
fn valid_schema_wire_format() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(Field::number(field_key!("age")))
        .add(
            Field::object(field_key!("address"))
                .add(Field::string(field_key!("city")))
                .add(Field::string(field_key!("zip")).required()),
        )
        .build()
        .unwrap();
    insta::assert_json_snapshot!(schema);
}

/// A `FieldValues` store covering every runtime-value shape (literal, nested
/// object, list, expression wrapper, mode envelope).
#[test]
fn field_values_wire_format() {
    let values = FieldValues::from_json(json!({
        "scalar": 1,
        "text": "hello",
        "flag": true,
        "nested": {"k": "v"},
        "list": [1, 2, 3],
        "expr": {"$expr": "{{ $x.y }}"},
        "mode": {"mode": "oauth2", "value": {"scope": "read"}}
    }))
    .unwrap();
    insta::assert_json_snapshot!(values);
}

/// Forward compatibility: a field whose `type` this version does not know
/// deserializes to `Field::Unknown` and re-serializes byte-for-byte, preserving
/// novel keys (`toolbar`) untouched. The snapshot freezes this preservation so a
/// future change that drops or reorders unknown keys diffs here and fails CI.
#[test]
fn unknown_field_type_preserved_verbatim() {
    let future_field = json!({
        "type": "richtext",
        "key": "bio",
        "label": "Biography",
        "visible": { "kind": "never" },
        "toolbar": ["bold", "italic"]
    });
    let field: Field =
        serde_json::from_value(future_field.clone()).expect("unknown type deserializes");
    assert!(matches!(field, Field::Unknown { .. }));
    assert_eq!(
        serde_json::to_value(&field).unwrap(),
        future_field,
        "an unknown field must round-trip verbatim"
    );
    insta::assert_json_snapshot!(field);
}

/// The typed `FieldValue::Mode` serialization branch. `from_json` parses a
/// `{"mode": …, "value": …}` object as an ordinary `Object`, so the `Mode`
/// variant's wire output is only reachable by constructing it directly.
#[test]
fn field_value_mode_wire_format() {
    let mode = FieldValue::Mode {
        mode: field_key!("oauth2"),
        value: Some(Box::new(FieldValue::from_json(json!({"scope": "read"})))),
    };
    insta::assert_json_snapshot!(mode);
}
