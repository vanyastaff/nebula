//! Declaration diagnostics never substitute for explicit wire disclosure.

use std::fmt::Debug;

use nebula_schema::{Field, Schema, SecretField, Transformer, ValidSchema, field_key};
use rstest::rstest;
use serde_json::{Value, json};

const PRIVATE: &str = "declaration-private-payload-74ca";

fn assert_private_debug(value: &impl Debug) {
    for diagnostic in [format!("{value:?}"), format!("{value:#?}")] {
        assert!(!diagnostic.contains(PRIVATE), "{diagnostic}");
    }
}

fn assert_typed_debug(field: &Field) {
    match field {
        Field::String(field) => assert_private_debug(field),
        Field::Secret(field) => assert_private_debug(field),
        Field::Number(field) => assert_private_debug(field),
        Field::Boolean(field) => assert_private_debug(field),
        Field::Select(field) => assert_private_debug(field),
        Field::Object(field) => assert_private_debug(field),
        Field::List(field) => assert_private_debug(field),
        Field::Mode(field) => assert_private_debug(field),
        Field::Code(field) => assert_private_debug(field),
        Field::File(field) => assert_private_debug(field),
        Field::Computed(field) => assert_private_debug(field),
        Field::Dynamic(field) => assert_private_debug(field),
        Field::Notice(field) => assert_private_debug(field),
        Field::Unknown(field) => assert_private_debug(field),
        _ => panic!("add a typed declaration privacy case for the new field kind"),
    }
}

#[rstest]
#[case::string(Field::string(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::secret(Field::secret(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::number(Field::number(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::boolean(Field::boolean(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::select(Field::select(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::object(Field::object(field_key!("field")).default(json!({"data": PRIVATE})).into())]
#[case::list(Field::list(field_key!("field")).default(json!([PRIVATE])).into())]
#[case::mode(Field::mode(field_key!("field")).default(json!({"mode": "token", "value": PRIVATE})).into())]
#[case::code(Field::code(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::file(Field::file(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::computed(Field::computed(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::dynamic(Field::dynamic(field_key!("field")).default(json!(PRIVATE)).into())]
#[case::notice(Field::notice(field_key!("field")).default(json!(PRIVATE)).into())]
fn defaults_are_private_before_lint_without_changing_wire_or_equality(#[case] field: Field) {
    let wire = serde_json::to_value(&field).unwrap();
    assert!(
        serde_json::to_string(&wire["default"])
            .unwrap()
            .contains(PRIVATE)
    );
    let decoded: Field = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, field);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
    for field in [field, decoded] {
        assert_typed_debug(&field);
        assert_private_debug(&field);
        let draft: Schema = serde_json::from_value(json!({"fields": [&field]})).unwrap();
        assert_private_debug(&draft);
        assert_private_debug(&Schema::builder().add(field));
    }
}

#[test]
fn mutated_secret_default_is_private_before_lint() {
    let mut field = SecretField::new(field_key!("token"));
    field.default = Some(json!({"nested": [PRIVATE]}));
    assert_private_debug(&field);
    assert_eq!(
        serde_json::to_value(&field).unwrap()["default"]["nested"][0],
        PRIVATE
    );
}

#[test]
fn secret_presentation_payloads_are_private_in_debug() {
    let field = Field::secret(field_key!("token"))
        .label(PRIVATE)
        .description(PRIVATE)
        .placeholder(PRIVATE)
        .group(PRIVATE);
    assert_private_debug(&field);
    let wire = serde_json::to_value(&field).unwrap();
    let decoded: SecretField = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, field);
    assert_private_debug(&decoded);
    assert_private_debug(&Schema::builder().add(field).build().unwrap());
    assert_eq!(wire["placeholder"], PRIVATE);
}

#[test]
fn replace_transformer_debug_is_private_but_wire_and_application_are_unchanged() {
    let transformer = Transformer::Replace {
        from: PRIVATE.to_owned(),
        to: format!("{PRIVATE}-new"),
    };
    assert_private_debug(&transformer);
    let wire = serde_json::to_value(&transformer).unwrap();
    let decoded: Transformer = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, transformer);
    assert_eq!(
        decoded.apply(&json!(PRIVATE)),
        json!(format!("{PRIVATE}-new"))
    );
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
    let schema = Schema::builder()
        .add(Field::secret(field_key!("token")).with_transformer(decoded))
        .build()
        .unwrap();
    assert_private_debug(&schema);
}

#[rstest]
#[case::missing(json!({"type": PRIVATE}))]
#[case::invalid(json!({"type": PRIVATE, "key": "bad key"}))]
#[case::wrong_type(json!({"type": PRIVATE, "key": 7}))]
fn unknown_invalid_key_diagnostics_do_not_echo_the_descriptor(#[case] wire: Value) {
    let error = serde_json::from_value::<Field>(wire).unwrap_err();
    assert_private_debug(&error);
    assert!(!error.to_string().contains(PRIVATE));
    assert!(error.to_string().contains("missing a valid"));
}

#[rstest]
#[case::widget(json!({"type": "secret", "key": "token", "widget": PRIVATE}))]
#[case::key(json!({"type": "secret", "key": PRIVATE}))]
#[case::reveal(json!({"type": "secret", "key": "token", "reveal_last": PRIVATE}))]
#[case::nested(json!({"type": "object", "key": "config", "fields": [
    {"type": "secret", "key": "token", "widget": PRIVATE}
]}))]
fn malformed_known_field_diagnostics_do_not_echo_payloads(#[case] wire: Value) {
    let error = serde_json::from_value::<Field>(wire).unwrap_err();
    assert_private_debug(&error);
    assert!(!error.to_string().contains(PRIVATE));
    assert!(error.to_string().contains("field descriptor"));
}

#[test]
fn malformed_widget_cannot_impersonate_a_transformer_diagnostic() {
    let error = serde_json::from_value::<Field>(json!({
        "type": "secret", "key": "token",
        "widget": format!("{PRIVATE} transformer.invalid_pattern")
    }))
    .unwrap_err();
    assert_private_debug(&error);
    assert!(!error.to_string().contains("transformer.invalid_pattern"));
}

#[rstest]
#[case::widget(json!({"key": "token", "widget": PRIVATE}))]
#[case::key(json!({"key": PRIVATE, "widget": "plain"}))]
#[case::reveal(json!({"key": "token", "widget": "plain", "reveal_last": PRIVATE}))]
fn direct_secret_descriptor_decode_errors_are_private(#[case] wire: Value) {
    let error = serde_json::from_value::<SecretField>(wire).unwrap_err();
    assert_private_debug(&error);
    assert!(!error.to_string().contains(PRIVATE));
    assert!(error.to_string().contains("field descriptor"));
}

fn secret_object() -> Field {
    Field::object(field_key!("config"))
        .add(
            Field::secret(field_key!("token"))
                .read_alias("old_token")
                .unwrap(),
        )
        .add(Field::string(field_key!("public")))
        .into()
}

fn secret_mode() -> nebula_schema::ModeField {
    Field::mode(field_key!("auth"))
        .variant("token", "Token", secret_object())
        .variant("public", "Public", Field::string(field_key!("text")))
}

#[rstest]
#[case::object(secret_object(), json!({"token": PRIVATE}))]
#[case::alias(secret_object(), json!({"old_token": PRIVATE}))]
#[case::losing_alias(secret_object(), json!({"token": null, "old_token": PRIVATE}))]
#[case::list(Field::list(field_key!("items")).item(secret_object()).into(), json!([{"token": PRIVATE}]))]
#[case::nested_list(Field::list(field_key!("items")).item(Field::list(field_key!("row")).item(Field::secret(field_key!("token")))).into(), json!([[PRIVATE]]))]
#[case::mode(secret_mode().into(), json!({"mode": "token", "value": {"token": PRIVATE}}))]
#[case::unknown_mode(secret_mode().into(), json!({"mode": "unknown", "value": PRIVATE}))]
#[case::malformed_object(secret_object(), json!(PRIVATE))]
fn secret_ancestor_defaults_cannot_reach_current_export_or_ui(
    #[case] field: Field,
    #[case] default: Value,
) {
    let mut field_wire = serde_json::to_value(field).unwrap();
    field_wire["default"] = default;
    let field: Field = serde_json::from_value(field_wire.clone()).unwrap();
    let report = Schema::builder().add(field).build().unwrap_err();
    let error = report
        .errors()
        .find(|error| error.code() == "secret.default_forbidden")
        .expect("secret defaults must be rejected at schema construction");
    assert_eq!(
        error.path().to_string(),
        format!("/{}", field_wire["key"].as_str().unwrap())
    );
    assert_private_debug(&report);
    assert!(!report.to_string().contains(PRIVATE));
    assert!(!serde_json::to_string(&report).unwrap().contains(PRIVATE));

    let current_wire = json!({"policy_version": 2, "fields": [&field_wire]});
    let current_error = serde_json::from_value::<ValidSchema>(current_wire).unwrap_err();
    assert!(
        current_error
            .to_string()
            .contains("secret.default_forbidden")
    );
    assert_private_debug(&current_error);

    let historical_wire = json!({"fields": [field_wire]});
    // Shape-invalid defaults were never historical schemas.
    if historical_wire["fields"][0]["default"].is_object()
        || historical_wire["fields"][0]["default"].is_array()
    {
        let historical: ValidSchema = serde_json::from_value(historical_wire.clone()).unwrap();
        assert_eq!(historical.policy_version(), 1);
        assert_eq!(serde_json::to_value(&historical).unwrap(), historical_wire);
        assert_private_debug(&historical);
        assert!(historical.ensure_current_semantics().is_err());
        #[cfg(feature = "schemars")]
        assert!(historical.json_schema().is_err());
    }
}

#[rstest]
#[case::list(Field::list(field_key!("items")).item(Field::secret(field_key!("token")).default(json!(PRIVATE))).into(), "/items/0")]
#[case::mode(Field::mode(field_key!("auth")).variant("token", "Token", Field::secret(field_key!("token")).default(json!(PRIVATE))).into(), "/auth/token")]
#[case::inactive(Field::mode(field_key!("auth")).variant_empty("none", "None").variant("token", "Token", Field::secret(field_key!("token")).default(json!(PRIVATE))).default_variant("none").into(), "/auth/token")]
fn anonymous_secret_defaults_are_linted_even_without_parent_defaults(
    #[case] field: Field,
    #[case] path: &str,
) {
    let historical_wire = json!({"fields": [&field]});
    let report = Schema::builder().add(field).build().unwrap_err();
    let error = report
        .errors()
        .find(|error| error.code() == "secret.default_forbidden")
        .unwrap();
    assert_eq!(error.path().to_string(), path);
    assert_private_debug(&report);
    let historical: ValidSchema = serde_json::from_value(historical_wire.clone()).unwrap();
    assert_eq!(historical.policy_version(), 1);
    assert_eq!(serde_json::to_value(&historical).unwrap(), historical_wire);
    assert_private_debug(&historical);
}

#[rstest]
#[case::empty_object(secret_object(), json!({}))]
#[case::null_secret(secret_object(), json!({"token": null}))]
#[case::public_only(secret_object(), json!({"public": "visible"}))]
#[case::empty_list(Field::list(field_key!("items")).item(secret_object()).into(), json!([]))]
#[case::public_mode(secret_mode().into(), json!({"mode": "public", "value": "visible"}))]
#[case::ordinary(Field::string(field_key!("name")).into(), json!("visible"))]
fn ordinary_defaults_keep_their_wire_and_export_values(
    #[case] field: Field,
    #[case] default: Value,
) {
    #[cfg(feature = "schemars")]
    let key = field.key().to_string();
    let mut wire = serde_json::to_value(field).unwrap();
    wire["default"] = default.clone();
    let field: Field = serde_json::from_value(wire.clone()).unwrap();
    let schema = Schema::builder().add(field).build().unwrap();
    assert_eq!(serde_json::to_value(&schema).unwrap()["fields"][0], wire);
    assert_eq!(schema.fields()[0].default(), Some(&default));
    #[cfg(feature = "schemars")]
    assert_eq!(
        serde_json::to_value(schema.json_schema().unwrap()).unwrap()["properties"][&key]["default"],
        default
    );
}
