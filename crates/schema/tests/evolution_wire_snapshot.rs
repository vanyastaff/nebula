//! Schema-evolution gate (C13): golden snapshots that **freeze the serde wire
//! representation** of the schema-definition types.
//!
//! `Property` is `#[serde(tag = "type")]` and is re-exported as a real external
//! contract (`nebula-api`'s public-schema projection consumes it), so a renamed
//! variant, a renamed field, or a changed type is a **silent** wire break that
//! no compiler catches. Any such change diffs a snapshot below and fails CI
//! until a maintainer consciously accepts it (`cargo insta review`).
//!
//! Backward/forward-compatibility rule this gate enforces by review:
//! - new `Property` variants are only safe because the enum is `#[non_exhaustive]`
//!   (an old reader must not be required to match them exhaustively);
//! - new struct fields must be `Option` / `#[serde(default)]` with
//!   `skip_serializing_if`, so a document written by an older version still
//!   deserializes and a document written by a newer version still round-trips;
//! - a `type` an old reader does not recognize deserializes to `Property::Unknown`
//!   and is preserved key-for-key (see `unknown_field_type_preserved`), so a
//!   newer writer's field kind never fails to read on an older deployment.
//!
//! Historical record/union vectors remain unversioned evidence. Separate current
//! writer vectors freeze policy v2; reading old bytes must not upgrade their policy.

use nebula_schema::{
    AuthoredValue, FieldPath, Predicate, Property, Rule, Schema, SerdeTagging, ValidSchema,
    ValidationError, ValidationReport, VisibilityMode, field_key,
};
use serde_json::json;

/// Every `Property` variant's wire shape (the `type` tag + each struct's
/// non-skipped fields). A renamed variant / field / type tag diffs here.
#[test]
fn field_variants_wire_format() {
    let variants: Vec<Property> = vec![
        // A fully-decorated field freezes the SHARED serde keys that are skipped
        // when default (`label`/`description`/`placeholder`/`default`/`group`/
        // `visible`/`rules`), so renaming any of them also diffs this snapshot.
        Property::string(field_key!("s"))
            .label("Display Name")
            .description("A fully-described field")
            .placeholder("type here")
            .group("contact")
            .default(json!("default-value"))
            .visible(VisibilityMode::Never)
            .with_rule(
                Rule::predicate(Predicate::eq("s", json!("x")).unwrap())
                    .expect("bounded snapshot rule"),
            )
            .required()
            .into(),
        Property::secret(field_key!("sec")).into(),
        Property::number(field_key!("n")).integer().into(),
        Property::boolean(field_key!("b")).into(),
        Property::select(field_key!("sel")).option("a", "A").into(),
        Property::object(field_key!("o"))
            .property(Property::string(field_key!("inner")))
            .into(),
        Property::list(field_key!("l"))
            .item(Property::string(field_key!("it")))
            .into(),
        Property::mode(field_key!("m"))
            .variant("v", "V", Property::string(field_key!("x")))
            .into(),
        Property::code(field_key!("c")).into(),
        Property::file(field_key!("f")).multiple().into(),
        Property::computed(field_key!("comp")).into(),
        Property::dynamic(field_key!("d")).into(),
        Property::notice(field_key!("not")).into(),
    ];
    insta::assert_json_snapshot!(variants);
}

fn read_historical_schema(wire: &str) -> ValidSchema {
    let schema: ValidSchema =
        serde_json::from_str(wire).expect("historical schema remains readable");
    assert_eq!(schema.policy_version(), 1);
    assert_eq!(
        serde_json::to_string(&schema).unwrap(),
        wire,
        "reading historical evidence must not upgrade or rewrite its bytes"
    );
    schema
}

/// The historical record vector retains its unversioned envelope and exact bytes.
#[test]
fn valid_schema_wire_format() {
    let schema = read_historical_schema(concat!(
        r#"{"fields":["#,
        r#"{"type":"string","key":"name","required":{"kind":"always"},"hint":"text","widget":"plain"},"#,
        r#"{"type":"number","key":"age","integer":false,"widget":"plain","step":null},"#,
        r#"{"type":"object","key":"address","fields":["#,
        r#"{"type":"string","key":"city","hint":"text","widget":"plain"},"#,
        r#"{"type":"string","key":"zip","required":{"kind":"always"},"hint":"text","widget":"plain"}"#,
        r#"],"widget":"inline"}]}"#,
    ));
    insta::assert_json_snapshot!(schema);
}

/// The same record declaration written freshly carries policy v2.
#[test]
fn current_valid_schema_wire_format() {
    let schema = Schema::builder()
        .property(Property::string(field_key!("name")).required())
        .property(Property::number(field_key!("age")))
        .property(
            Property::object(field_key!("address"))
                .property(Property::string(field_key!("city")))
                .property(Property::string(field_key!("zip")).required()),
        )
        .build()
        .unwrap();
    assert_eq!(schema.policy_version(), 2);
    insta::assert_json_snapshot!(schema);
}

/// Both historical serde tagging forms retain their original envelopes and bytes.
#[test]
fn union_schema_wire_format() {
    let external = read_historical_schema(concat!(
        r#"{"kind":"union","serde_tagging":"external","fields":["#,
        r#"{"type":"mode","key":"auth","required":{"kind":"always"},"variants":["#,
        r#"{"key":"oauth","label":"OAuth","field":{"type":"object","key":"oauth","fields":["#,
        r#"{"type":"secret","key":"token","required":{"kind":"always"},"widget":"plain","reveal_last":null}"#,
        r#"],"widget":"inline"}},"#,
        r#"{"key":"none","label":"None","field":{"type":"string","key":"_nebula_mode_empty","visible":{"kind":"never"},"expression":"forbidden","hint":"text","widget":"plain"}}"#,
        r#"],"default_variant":null}]}"#,
    ));
    insta::assert_json_snapshot!("union_schema_external_wire_format", external);

    let adjacent = read_historical_schema(concat!(
        r#"{"kind":"union","serde_tagging":{"adjacent":{"tag":"type","content":"data"}},"fields":["#,
        r#"{"type":"mode","key":"event","required":{"kind":"always"},"variants":["#,
        r#"{"key":"click","label":"Click","field":{"type":"object","key":"click","fields":["#,
        r#"{"type":"number","key":"x","required":{"kind":"always"},"integer":false,"widget":"plain","step":null}"#,
        r#"],"widget":"inline"}},"#,
        r#"{"key":"noop","label":"No-op","field":{"type":"string","key":"_nebula_mode_empty","visible":{"kind":"never"},"expression":"forbidden","hint":"text","widget":"plain"}}"#,
        r#"],"default_variant":null}]}"#,
    ));
    insta::assert_json_snapshot!("union_schema_adjacent_wire_format", adjacent);
}

/// Fresh tagged unions retain their serde tagging and explicitly write policy v2.
#[test]
fn current_union_schema_wire_format() {
    let external = ValidSchema::union(
        Property::mode(field_key!("auth"))
            .variant(
                "oauth",
                "OAuth",
                Property::object(field_key!("oauth"))
                    .property(Property::secret(field_key!("token")).required()),
            )
            .variant_empty("none", "None"),
        SerdeTagging::External,
    )
    .unwrap();
    assert_eq!(external.policy_version(), 2);
    insta::assert_json_snapshot!("current_union_schema_external_wire_format", external);

    let adjacent = ValidSchema::union(
        Property::mode(field_key!("event"))
            .variant(
                "click",
                "Click",
                Property::object(field_key!("click"))
                    .property(Property::number(field_key!("x")).required()),
            )
            .variant_empty("noop", "No-op"),
        SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "data".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(adjacent.policy_version(), 2);
    insta::assert_json_snapshot!("current_union_schema_adjacent_wire_format", adjacent);
}

/// An authored tree covering every value shape (literal, nested
/// object, list, expression wrapper, mode envelope).
#[test]
fn field_values_wire_format() {
    let values = AuthoredValue::from_template_json(json!({
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
/// deserializes to `Property::Unknown` and re-serializes with every key and value
/// preserved, including novel keys (`toolbar`). The snapshot freezes this
/// preservation so a future change that *drops or renames* a preserved key diffs
/// here and fails CI. (Key *order* is normalized by serde_json — the snapshot is
/// alphabetized — so a reorder is intentionally not asserted.)
#[test]
fn unknown_field_type_preserved() {
    let future_field = json!({
        "type": "richtext",
        "key": "bio",
        "label": "Biography",
        "visible": { "kind": "never" },
        "toolbar": ["bold", "italic"]
    });
    let field: Property =
        serde_json::from_value(future_field.clone()).expect("unknown type deserializes");
    assert!(matches!(field, Property::Unknown(_)));
    assert_eq!(
        serde_json::to_value(&field).unwrap(),
        future_field,
        "every key and value of an unknown field must round-trip"
    );
    insta::assert_json_snapshot!(field);
}

/// A mode envelope is an ordinary object in the single value-tree representation.
#[test]
fn field_value_mode_wire_format() {
    let mode =
        AuthoredValue::from_data(json!({"mode": "oauth2", "value": {"scope": "read"}})).unwrap();
    insta::assert_json_snapshot!(mode);
}

/// The serde wire shape of the structured error types. `ValidationError`'s `code`
/// is the stable machine-readable vocabulary and the type is now sent over the
/// wire (API responses, cross-process), so a renamed field, a changed `severity`
/// tag, or a newly serialized field is a silent wire break that diffs here. The
/// report serializes transparently (a flat array); `source` is never on the wire.
#[test]
fn validation_report_wire_format() {
    let mut report = ValidationReport::new();
    report.push(
        ValidationError::builder("length.max")
            .at(FieldPath::parse("user.tags[0].name").unwrap())
            .message("value too long")
            .param("max", json!(20))
            .param("actual", json!(42))
            .build(),
    );
    report.push(
        ValidationError::builder("notice.deprecated")
            .warn()
            .message("field is deprecated")
            .build(),
    );
    insta::assert_json_snapshot!(report);
}

/// Literal input projection exercising every registered export extension.
#[cfg(feature = "schemars")]
fn literal_extension_fixture() -> ValidSchema {
    let enabled = Rule::predicate(Predicate::eq("/enabled", json!(true)).unwrap()).unwrap();
    Schema::builder()
        .property(Property::boolean(field_key!("enabled")))
        .property(
            Property::string(field_key!("name"))
                .no_expression()
                .required()
                .min_length(3)
                .max_length(32)
                .label("Display name")
                .description("Literal name with inbound aliases")
                .default(json!("Example"))
                .read_alias("legacy_name")
                .unwrap()
                .read_alias("old_name")
                .unwrap()
                .emit_as("display_name")
                .unwrap(),
        )
        .property(
            Property::file(field_key!("avatar"))
                .no_expression()
                .accept("image/png")
                .max_size(1_048_576),
        )
        .property(
            Property::file(field_key!("attachments"))
                .no_expression()
                .multiple()
                .accept("application/pdf,image/*")
                .max_size(0),
        )
        .property(
            Property::select(field_key!("regions"))
                .dynamic()
                .multiple()
                .allow_custom(),
        )
        .property(Property::select(field_key!("provider")).extend_options([
            nebula_schema::SelectOption::new(json!("current"), "Current"),
            nebula_schema::SelectOption::new(json!("legacy"), "Legacy").disabled(),
        ]))
        .property(
            Property::select(field_key!("tags"))
                .multiple()
                .extend_options([
                    nebula_schema::SelectOption::new(json!("stable"), "Stable"),
                    nebula_schema::SelectOption::new(json!("retired"), "Retired").disabled(),
                ]),
        )
        .property(
            Property::mode(field_key!("auth"))
                .no_expression()
                .variant_empty("none", "None")
                .variant(
                    "token",
                    "Token",
                    Property::string(field_key!("token"))
                        .required()
                        .no_expression(),
                )
                .default_variant("none"),
        )
        .property(
            Property::object(field_key!("profile"))
                .no_expression()
                .property(Property::string(field_key!("note")).no_expression()),
        )
        .property(
            Property::list(field_key!("labels"))
                .no_expression()
                .item(Property::string(field_key!("label")).no_expression()),
        )
        .property(
            Property::string(field_key!("conditional"))
                .no_expression()
                .active_when(enabled.clone()),
        )
        .property(
            Property::string(field_key!("hidden"))
                .no_expression()
                .required()
                .visible(VisibilityMode::Never),
        )
        .root_rule(enabled)
        .build()
        .expect("literal extension fixture has valid declarations")
}

#[cfg(feature = "schemars")]
#[test]
fn json_schema_literal_extensions() {
    let exported = literal_extension_fixture()
        .json_schema()
        .expect("literal fixture exports")
        .to_value();
    insta::assert_json_snapshot!("json_schema_literal_extensions", exported);
}

#[cfg(feature = "schemars")]
#[test]
fn json_schema_expression_modes() {
    let schema = Schema::builder()
        .property(
            Property::string(field_key!("literal"))
                .no_expression()
                .min_length(2),
        )
        .property(Property::string(field_key!("template")).min_length(2))
        .property(
            Property::computed(field_key!("computed"))
                .returns(nebula_schema::ComputedReturn::Number),
        )
        .build()
        .expect("expression mode fixture has valid declarations");
    let exported = schema
        .json_schema()
        .expect("expression fixture exports")
        .to_value();
    insta::assert_json_snapshot!("json_schema_expression_modes", exported);
}
