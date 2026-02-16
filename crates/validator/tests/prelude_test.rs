//! Integration tests for the prelude module.
//!
//! Verifies that `use nebula_validator::prelude::*` brings in everything
//! a consumer needs for common validation scenarios.

#![cfg(feature = "serde")]

use nebula_validator::prelude::*;
use serde_json::json;

// ============================================================================
// PRELUDE IMPORT SMOKE TEST
// ============================================================================

#[test]
fn prelude_import_provides_validate_trait() {
    // Verify Validate and ValidateExt are available through the prelude.
    let v = min_length(3).and(max_length(20));
    assert!(v.validate("hello").is_ok());
    assert!(v.validate("hi").is_err());
}

// ============================================================================
// STRING VALIDATOR VIA PRELUDE
// ============================================================================

#[test]
fn min_length_via_prelude() {
    let v = min_length(5);
    assert!(v.validate("hello").is_ok());
    assert!(v.validate("hi").is_err());
}

#[test]
fn validate_any_string_from_json() {
    let v = min_length(5);
    assert!(v.validate_any(&json!("hello")).is_ok());
    assert!(v.validate_any(&json!("hi")).is_err());
}

// ============================================================================
// JSON SIZE VALIDATORS (TURBOFISH-FREE)
// ============================================================================

#[test]
fn json_min_size_without_turbofish() {
    let v = json_min_size(2);
    assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(v.validate_any(&json!([1])).is_err());
}

#[test]
fn json_max_size_without_turbofish() {
    let v = json_max_size(3);
    assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(v.validate_any(&json!([1, 2, 3, 4])).is_err());
}

#[test]
fn json_exact_size_without_turbofish() {
    let v = json_exact_size(2);
    assert!(v.validate_any(&json!([1, 2])).is_ok());
    assert!(v.validate_any(&json!([1])).is_err());
}

#[test]
fn json_size_range_without_turbofish() {
    let v = json_size_range(2, 4);
    assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    assert!(v.validate_any(&json!([])).is_err());
    assert!(v.validate_any(&json!([1, 2, 3, 4, 5])).is_err());
}

// ============================================================================
// TYPE MISMATCH ERRORS
// ============================================================================

#[test]
fn type_mismatch_on_null_for_string_validator() {
    let v = min_length(1);
    let err = v.validate_any(&json!(null)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
}

#[test]
fn type_mismatch_on_null_for_array_validator() {
    let v = json_min_size(1);
    let err = v.validate_any(&json!(null)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
}

#[test]
fn type_mismatch_on_number_for_string_validator() {
    let v = min_length(1);
    let err = v.validate_any(&json!(42)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert!(err.message.contains("string"));
}

// ============================================================================
// COMBINATORS VIA PRELUDE
// ============================================================================

#[test]
fn and_combinator_via_prelude() {
    let v = min_length(3).and(max_length(10));
    assert!(v.validate("hello").is_ok());
    assert!(v.validate("hi").is_err());
}

#[test]
fn or_combinator_via_prelude() {
    let v = exact_length(5).or(exact_length(10));
    assert!(v.validate("hello").is_ok());
    assert!(v.validate("hi").is_err());
}

#[test]
fn not_combinator_via_prelude() {
    let v = not(contains("bad"));
    assert!(v.validate("good").is_ok());
    assert!(v.validate("bad word").is_err());
}

#[test]
fn json_field_via_prelude() {
    let v = json_field("/name", min_length(1));
    assert!(v.validate(&json!({"name": "Alice"})).is_ok());
    assert!(v.validate(&json!({"name": ""})).is_err());
}

#[test]
fn json_field_optional_via_prelude() {
    let v = json_field_optional("/email", min_length(5));
    assert!(v.validate(&json!({"name": "Alice"})).is_ok());
    assert!(v.validate(&json!({"email": "a@b.c"})).is_ok());
}

// ============================================================================
// COMPOSED VALIDATOR SCENARIO
// ============================================================================

#[test]
fn composed_json_validation_via_prelude() {
    let v = json_field("/name", min_length(1))
        .and(json_field("/tags", json_min_size(1)))
        .and(json_field_optional("/email", email()));

    let valid = json!({
        "name": "Alice",
        "tags": ["admin", "user"],
        "email": "alice@example.com"
    });
    assert!(v.validate(&valid).is_ok());

    // Missing name
    let err = v
        .validate(&json!({
            "name": "",
            "tags": ["admin"],
        }))
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("/name"));
}
