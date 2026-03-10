use nebula_validator::foundation::{ErrorSeverity, ValidationError};
use serde_json::Value;

/// Required top-level keys that must always be present in a serialized envelope.
const ENVELOPE_REQUIRED_KEYS: &[&str] = &["code", "message"];

#[test]
fn error_envelope_contains_required_contract_fields() {
    let error = ValidationError::new("min_length", "Must be at least 3 characters")
        .with_field("user.name")
        .with_param("min", "3")
        .with_severity(ErrorSeverity::Error);

    let json = error.to_json_value();
    for key in ENVELOPE_REQUIRED_KEYS {
        assert!(
            json.get(key).is_some(),
            "envelope missing required key: {key}"
        );
    }
}

#[test]
fn contract_schema_declares_recursive_nested_structure() {
    let schema_raw = include_str!(
        "../../../../specs/001-validator-crate-spec/contracts/validation-error-envelope.schema.json"
    );
    let schema: Value = serde_json::from_str(schema_raw).expect("schema JSON must parse");

    assert_eq!(
        schema
            .get("type")
            .and_then(Value::as_str)
            .expect("schema type must exist"),
        "object"
    );

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("required array must exist");
    assert!(required.iter().any(|v| v == "code"));
    assert!(required.iter().any(|v| v == "message"));

    let nested_ref = schema
        .get("properties")
        .and_then(|p| p.get("nested"))
        .and_then(|n| n.get("items"))
        .and_then(|i| i.get("$ref"))
        .and_then(Value::as_str)
        .expect("nested.items.$ref must exist");
    assert_eq!(nested_ref, "#");
}

#[test]
fn envelope_fixtures_match_serialized_shape() {
    #[derive(serde::Deserialize)]
    struct EnvelopeFixture {
        id: String,
        #[expect(dead_code, reason = "informational only")]
        scenario: String,
        envelope: Value,
        required_keys: Vec<String>,
    }

    let raw = include_str!("../fixtures/compat/envelope_contract_v1.json");
    let fixtures: Vec<EnvelopeFixture> =
        serde_json::from_str(raw).expect("envelope fixture JSON must be valid");

    for fixture in &fixtures {
        let envelope = fixture.envelope.as_object().unwrap_or_else(|| {
            panic!("fixture {} envelope must be an object", fixture.id);
        });

        for key in &fixture.required_keys {
            assert!(
                envelope.contains_key(key.as_str()),
                "fixture {} missing required key '{}' in frozen envelope",
                fixture.id,
                key
            );
        }
    }
}

#[test]
fn serialized_envelope_always_contains_code_and_message() {
    let cases = [
        ValidationError::new("required", "value is required"),
        ValidationError::new("min_length", "too short").with_field("name"),
        ValidationError::new("or_failed", "both failed")
            .with_nested_error(ValidationError::new("a", "left"))
            .with_nested_error(ValidationError::new("b", "right")),
    ];

    for error in &cases {
        let json = error.to_json_value();
        let obj = json.as_object().expect("must serialize as object");

        assert!(
            obj.contains_key("code"),
            "'code' missing in envelope for: {}",
            error.code
        );
        assert!(
            obj.contains_key("message"),
            "'message' missing in envelope for: {}",
            error.code
        );
    }
}
