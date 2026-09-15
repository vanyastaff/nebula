use nebula_schema::{
    AuthoredValue, Property, ScalarSchema, Schema, SerdeTagging, ValidSchema, VisibilityMode,
    field_key,
};
use serde_json::json;

#[test]
fn fresh_schema_records_current_policy() {
    let wire = serde_json::to_value(ValidSchema::empty()).unwrap();
    assert_eq!(wire["policy_version"], json!(2));
}

#[test]
fn malformed_policy_diagnostics_do_not_echo_marker_payloads() {
    const CANARY: &str = "private-policy-marker";
    for marker in [json!(CANARY), json!([CANARY]), json!({"value": CANARY})] {
        let wire = json!({"policy_version": marker, "fields": []});
        for error in [
            serde_json::from_value::<ValidSchema>(wire.clone()).unwrap_err(),
            serde_json::from_str::<ValidSchema>(&wire.to_string()).unwrap_err(),
        ] {
            let mut cause: Option<&dyn std::error::Error> = Some(&error);
            while let Some(error) = cause {
                assert!(!error.to_string().contains(CANARY));
                assert!(!format!("{error:?}").contains(CANARY));
                cause = error.source();
            }
        }
    }
}

#[test]
fn historical_schema_is_evidence_not_value_authority() {
    let wire = r#"{"fields":[]}"#;
    let historical: ValidSchema = serde_json::from_str(wire).unwrap();
    assert_eq!(serde_json::to_string(&historical).unwrap(), wire);
    let report = historical
        .validate(AuthoredValue::from_data(json!({})).unwrap())
        .expect_err("historical evidence cannot validate current values");
    assert!(
        report
            .errors()
            .any(|error| error.code() == "schema.unsupported_policy")
    );
}

#[test]
fn identical_roots_with_different_policies_are_not_equal() {
    let historical: ValidSchema = serde_json::from_value(json!({"fields": []})).unwrap();
    assert_ne!(historical, ValidSchema::empty());
}

#[test]
fn newtype_payload_extraction_cannot_promote_historical_schema() {
    let historical: ValidSchema = serde_json::from_str(r#"{"fields":[]}"#).unwrap();
    let report = nebula_schema::__private::union_newtype_payload(
        field_key!("payload"),
        historical,
        "Outer",
        "Variant",
    )
    .unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| error.code() == "schema.unsupported_policy")
    );
}

#[test]
fn presentation_cannot_waive_current_requiredness() {
    let schema = Schema::builder()
        .property(
            Property::string(field_key!("value"))
                .required()
                .visible(VisibilityMode::Never),
        )
        .build()
        .unwrap();
    let report = schema
        .validate(AuthoredValue::from_data(json!({})).unwrap())
        .expect_err("hidden required property must still be supplied");
    assert!(report.errors().any(|error| error.code() == "required"));
}

#[test]
fn historical_hidden_required_policy_is_preserved_without_reinterpretation() {
    let current = Schema::builder()
        .property(
            Property::string(field_key!("value"))
                .required()
                .visible(VisibilityMode::Never),
        )
        .property(
            Property::mode(field_key!("mode")).variant(
                "hidden",
                "Hidden",
                Property::string(field_key!("payload"))
                    .required()
                    .visible(VisibilityMode::Never),
            ),
        )
        .build()
        .unwrap();
    let mut historical_wire = serde_json::to_value(&current).unwrap();
    historical_wire
        .as_object_mut()
        .unwrap()
        .remove("policy_version");
    let historical: ValidSchema = serde_json::from_value(historical_wire.clone()).unwrap();
    let canonical_bytes = serde_json::to_vec(&historical).unwrap();
    let decoded: ValidSchema = serde_json::from_slice(&canonical_bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), canonical_bytes);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), historical_wire);
    assert_eq!(decoded.root_shape(), current.root_shape());
    assert_ne!(decoded, current);
    for input in [
        json!({}),
        json!({"value":"supplied","mode":{"mode":"hidden","value":"supplied"}}),
    ] {
        let report = decoded
            .validate(AuthoredValue::from_data(input).unwrap())
            .unwrap_err();
        assert!(
            report
                .errors()
                .any(|error| error.code() == "schema.unsupported_policy")
        );
    }
}

#[cfg(feature = "schemars")]
#[test]
fn historical_schema_cannot_export_current_contract() {
    let historical: ValidSchema = serde_json::from_value(json!({"fields": []})).unwrap();
    assert!(historical.json_schema().is_err());
}

#[test]
fn all_roots_preserve_policy_through_serde_without_promoting_legacy_evidence() {
    let union = ValidSchema::union(
        Property::mode(field_key!("choice")).variant_empty("none", "None"),
        SerdeTagging::External,
    )
    .unwrap();
    for current in [
        ValidSchema::empty(),
        ValidSchema::any(),
        ValidSchema::scalar(ScalarSchema::null()).unwrap(),
        union,
    ] {
        current.ensure_current_semantics().unwrap();
        let current_wire = serde_json::to_value(&current).unwrap();
        assert_eq!(current_wire["policy_version"], json!(2));
        let decoded: ValidSchema = serde_json::from_value(current_wire.clone()).unwrap();
        assert_eq!(decoded, current);
        assert_eq!(decoded.policy_version(), 2);

        let mut historical_wire = current_wire;
        historical_wire
            .as_object_mut()
            .unwrap()
            .remove("policy_version");
        let historical: ValidSchema = serde_json::from_value(historical_wire.clone()).unwrap();
        assert_eq!(historical.policy_version(), 1);
        assert_ne!(historical, current);
        assert_eq!(historical.root_shape(), current.root_shape());
        assert_eq!(serde_json::to_value(&historical).unwrap(), historical_wire);
        assert!(historical.ensure_current_semantics().is_err());
        assert_eq!(
            current.policy_version(),
            2,
            "legacy decoding must not mutate shared current roots"
        );
        assert!(
            historical
                .validate(AuthoredValue::from_data(json!({})).unwrap())
                .is_err()
        );
        #[cfg(feature = "schemars")]
        assert!(historical.json_schema().is_err());
    }
}

#[test]
fn schema_policy_marker_is_closed_and_cannot_be_null_or_defaulted() {
    for policy in [
        json!(null),
        json!(0),
        json!(1),
        json!(3),
        json!(2.0),
        json!("2"),
        json!({}),
        json!([]),
    ] {
        assert!(
            serde_json::from_value::<ValidSchema>(json!({"policy_version": policy, "fields": []}))
                .is_err()
        );
    }
    assert!(
        serde_json::from_str::<ValidSchema>(
            r#"{"policy_version":2,"policy_version":2,"fields":[]}"#
        )
        .is_err()
    );
}

#[test]
fn legacy_schema_cannot_prove_current_assignability_even_for_any() {
    use nebula_schema::{
        Assignability, InputSchema, OutputSchema, UnknownReason, explain_assignable,
    };
    let historical: ValidSchema = serde_json::from_value(json!({"fields": []})).unwrap();
    let verdict = explain_assignable(
        &OutputSchema::new(historical),
        &InputSchema::new(ValidSchema::any()),
    );
    assert_eq!(
        verdict,
        Assignability::Unknown(vec![UnknownReason::UnsupportedPolicy])
    );
}
