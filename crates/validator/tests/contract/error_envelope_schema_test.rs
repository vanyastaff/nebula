use nebula_validator::foundation::{ErrorSeverity, ValidationError};
use serde_json::Value;

#[test]
fn error_envelope_contains_required_contract_fields() {
    let error = ValidationError::new("min_length", "Must be at least 3 characters")
        .with_field("user.name")
        .with_param("min", "3")
        .with_severity(ErrorSeverity::Error);

    let json = error.to_json_value();
    assert!(json.get("code").is_some());
    assert!(json.get("message").is_some());
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
