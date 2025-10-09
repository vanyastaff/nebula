//! Quick test to verify new validators work without building nebula-parameter

use nebula_validator::core::TypedValidator;
use nebula_validator::validators::text::*;

#[test]
fn test_uuid_validator() {
    let validator = Uuid::new();
    assert!(
        validator
            .validate("550e8400-e29b-41d4-a716-446655440000")
            .is_ok()
    );
    assert!(validator.validate("not-a-uuid").is_err());
}

#[test]
fn test_datetime_validator() {
    let validator = DateTime::new();
    assert!(validator.validate("2024-01-15T10:30:00Z").is_ok());
    assert!(validator.validate("not-a-date").is_err());
}

#[test]
fn test_json_validator() {
    let validator = Json::new();
    assert!(validator.validate(r#"{"key": "value"}"#).is_ok());
    assert!(validator.validate("not json").is_err());
}

#[test]
fn test_slug_validator() {
    let validator = Slug::new();
    assert!(validator.validate("my-awesome-slug").is_ok());
    assert!(validator.validate("Not A Slug!").is_err());
}

#[test]
fn test_hex_validator() {
    let validator = Hex::new();
    assert!(validator.validate("deadbeef").is_ok());
    assert!(validator.validate("not hex").is_err());
}

#[test]
fn test_base64_validator() {
    let validator = Base64::new();
    assert!(validator.validate("SGVsbG8gV29ybGQ=").is_ok());
    assert!(validator.validate("not base64!").is_err());
}
