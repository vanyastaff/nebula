//! Integration tests for serde_json::Value validation.

#![cfg(feature = "serde")]

use nebula_validator::combinators::{json_field, json_field_optional};
use nebula_validator::core::{Validate, ValidateExt};
use nebula_validator::validators::string::min_length;
use serde_json::json;

#[test]
fn validate_string_value_directly() {
    let validator = min_length(3);
    assert!(validator.validate_any(&json!("hello")).is_ok());
    assert!(validator.validate_any(&json!("hi")).is_err());
}

#[test]
fn validate_config_structure() {
    let data = json!({
        "server": {
            "host": "localhost",
            "port": 8080
        },
        "database": {
            "url": "postgres://localhost/db"
        }
    });

    let host = json_field("/server/host", min_length(1));
    let db_url = json_field("/database/url", min_length(5));
    let combined = host.and(db_url);
    assert!(combined.validate(&data).is_ok());
}

#[test]
fn validate_array_element_by_index() {
    let data = json!({
        "servers": [
            {"host": "web1", "port": 80},
            {"host": "web2", "port": 443}
        ]
    });

    let first_host = json_field("/servers/0/host", min_length(1));
    assert!(first_host.validate(&data).is_ok());

    let second_host = json_field("/servers/1/host", min_length(1));
    assert!(second_host.validate(&data).is_ok());
}

#[test]
fn optional_field_missing() {
    let data = json!({"name": "Alice"});
    let optional = json_field_optional("/email", min_length(5));
    assert!(optional.validate(&data).is_ok());
}

#[test]
fn type_mismatch_gives_clear_error() {
    let validator = min_length(1);
    let err = validator.validate_any(&json!(42)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert!(err.message.contains("string"));
}

#[test]
fn null_value_type_mismatch() {
    let validator = min_length(1);
    let err = validator.validate_any(&json!(null)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
}

#[test]
fn min_size_with_json_array() {
    use nebula_validator::validators::collection::min_size;

    let validator = min_size::<serde_json::Value>(2);
    assert!(validator.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(validator.validate_any(&json!([1])).is_err());
}

#[test]
fn composed_json_field_validators() {
    let data = json!({
        "user": {
            "name": "Alice",
            "age": 28
        },
        "settings": {
            "theme": "dark"
        }
    });

    let v =
        json_field("/user/name", min_length(1)).and(json_field("/settings/theme", min_length(1)));

    assert!(v.validate(&data).is_ok());
}
