//! Unknown declarations remain readable evidence, never executable contracts.

use nebula_schema::{AuthoredValue, Field, Schema, ValidSchema, field_key};
use rstest::rstest;
use serde_json::{Value, json};

fn unknown() -> Field {
    serde_json::from_value(json!({
        "type": "vendor.future_secret",
        "key": "future",
        "vendor_payload": "descriptor-sentinel"
    }))
    .unwrap()
}

#[test]
fn unknown_descriptor_debug_is_redacted_without_losing_explicit_wire() {
    let field = unknown();
    let wire = serde_json::to_value(&field).unwrap();
    assert_eq!(wire["vendor_payload"], "descriptor-sentinel");
    let field_debug = format!("{field:?}");
    let schema = Schema::builder().add(field).build().unwrap();
    for diagnostic in [field_debug, format!("{schema:?}")] {
        assert!(!diagnostic.contains("descriptor-sentinel"));
        assert!(!diagnostic.contains("vendor.future_secret"));
    }
    assert_eq!(serde_json::to_value(&schema).unwrap()["fields"][0], wire);
}

#[rstest]
#[case::root(unknown(), "/future")]
#[case::object(Field::object(field_key!("config")).add(unknown()).into(), "/config/future")]
#[case::list(Field::list(field_key!("items")).item(unknown()).into(), "/items/0")]
#[case::nested_list(Field::list(field_key!("items")).item(Field::list(field_key!("row")).item(unknown())).into(), "/items/0/0")]
#[case::mode(Field::mode(field_key!("auth")).variant_empty("none", "None").variant("future", "Future", unknown()).into(), "/auth/future")]
fn unknown_kind_cannot_produce_value_proof(#[case] field: Field, #[case] path: &str) {
    let schema = Schema::builder().add(field).build().unwrap();
    let wire = serde_json::to_value(&schema).unwrap();
    let historical: ValidSchema = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&historical).unwrap(), wire);

    for input in [json!({}), json!({"auth": {"mode": "none"}})] {
        let report = historical
            .validate(AuthoredValue::from_data(input).unwrap())
            .expect_err("an absent or inactive unknown property still lacks a validator");
        let error = report.errors().next().unwrap();
        assert_eq!(error.code(), "schema.unsupported_property_kind");
        assert_eq!(error.path().to_string(), path);
        assert!(!format!("{report:?}").contains("descriptor-sentinel"));
        assert!(!format!("{report}").contains("vendor.future_secret"));
    }
}

#[test]
fn explicit_json_escape_hatch_still_validates_literal_data() {
    let schema = nebula_schema::schema_of::<Value>().unwrap();
    let input = json!({"kind": "data", "nested": [1, null, true]});
    let resolved = schema
        .validate(AuthoredValue::from_data(input.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.into_json(), input);
}

#[cfg(feature = "schemars")]
#[rstest]
#[case::root(unknown())]
#[case::list(Field::list(field_key!("items")).item(unknown()).into())]
#[case::inactive_mode(Field::mode(field_key!("auth")).variant_empty("none", "None").variant("future", "Future", unknown()).into())]
fn unknown_kind_cannot_export_as_unconstrained_json_schema(#[case] field: Field) {
    let schema = Schema::builder().add(field).build().unwrap();
    let error = schema
        .json_schema()
        .expect_err("unknown semantics must not export an unconstrained schema");
    assert!(error.to_string().contains("unsupported property kind"));
    assert!(!format!("{error:?}").contains("descriptor-sentinel"));
    assert!(!error.to_string().contains("vendor.future_secret"));
}
