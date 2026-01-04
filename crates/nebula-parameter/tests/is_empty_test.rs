//! Tests for is_empty behavior across parameter types
//!
//! This test module verifies that all parameter types that work with strings or collections
//! properly implement `is_empty()` in their `Validatable` impl to ensure required field
//! validation correctly rejects empty values.

use nebula_parameter::prelude::*;
use nebula_value::Value;
use nebula_value::collections::Array;

// =============================================================================
// String Types - is_empty tests
// =============================================================================

#[test]
fn test_text_is_empty() {
    let param = TextParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::text("")),
        "Empty string should be empty"
    );
    assert!(
        !param.is_empty(&Value::text("hello")),
        "Non-empty string should NOT be empty"
    );
    assert!(
        !param.is_empty(&Value::text("   ")),
        "Whitespace-only string should NOT be empty (not trimmed)"
    );
}

#[test]
fn test_textarea_is_empty() {
    let param = TextareaParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::text("")),
        "Empty string should be empty"
    );
    assert!(
        !param.is_empty(&Value::text("content")),
        "Non-empty string should NOT be empty"
    );
    assert!(
        !param.is_empty(&Value::text("   ")),
        "Whitespace-only string should NOT be empty (not trimmed)"
    );
}

#[test]
fn test_secret_is_empty() {
    let param = SecretParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::text("")),
        "Empty string should be empty"
    );
    assert!(
        !param.is_empty(&Value::text("secret123")),
        "Non-empty string should NOT be empty"
    );
}

#[test]
fn test_code_is_empty() {
    let param = CodeParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::text("")),
        "Empty string should be empty"
    );
    assert!(
        !param.is_empty(&Value::text("console.log('hi')")),
        "Non-empty string should NOT be empty"
    );
    assert!(
        !param.is_empty(&Value::text("   ")),
        "Whitespace-only string should NOT be empty (not trimmed)"
    );
}

#[test]
fn test_color_is_empty() {
    let param = ColorParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::text("")),
        "Empty string should be empty"
    );
    assert!(
        !param.is_empty(&Value::text("#FF0000")),
        "Non-empty string should NOT be empty"
    );
}

// =============================================================================
// Collection Types - is_empty tests
// =============================================================================

#[test]
fn test_list_is_empty() {
    let param = ListParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::array_empty()),
        "Empty array should be empty"
    );

    let non_empty_array = Value::Array(Array::from_vec(vec![Value::integer(1)]));
    assert!(
        !param.is_empty(&non_empty_array),
        "Non-empty array should NOT be empty"
    );
}

#[test]
fn test_object_is_empty() {
    let param = ObjectParameter::builder()
        .key("test")
        .name("Test")
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::object_empty()),
        "Empty object should be empty"
    );
}

#[test]
fn test_group_is_empty() {
    let param = GroupParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::object_empty()),
        "Empty object should be empty"
    );
}

#[test]
fn test_multi_select_is_empty() {
    let param = MultiSelectParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    assert!(param.is_empty(&Value::Null), "Null should be empty");
    assert!(
        param.is_empty(&Value::array_empty()),
        "Empty array should be empty"
    );

    let non_empty_array = Value::Array(Array::from_vec(vec![Value::text("option1")]));
    assert!(
        !param.is_empty(&non_empty_array),
        "Non-empty array should NOT be empty"
    );
}

// =============================================================================
// Required Validation - Integration tests
// =============================================================================

#[tokio::test]
async fn test_required_text_rejects_empty_string() {
    let param = TextParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::text("")).await;
    assert!(result.is_err(), "Required text should reject empty string");

    let result = param.validate(&Value::Null).await;
    assert!(result.is_err(), "Required text should reject Null");

    let result = param.validate(&Value::text("hello")).await;
    assert!(
        result.is_ok(),
        "Required text should accept non-empty string"
    );
}

#[tokio::test]
async fn test_required_textarea_rejects_empty_string() {
    let param = TextareaParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::text("")).await;
    assert!(
        result.is_err(),
        "Required textarea should reject empty string"
    );

    let result = param.validate(&Value::text("content")).await;
    assert!(
        result.is_ok(),
        "Required textarea should accept non-empty string"
    );
}

#[tokio::test]
async fn test_required_secret_rejects_empty_string() {
    let param = SecretParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::text("")).await;
    assert!(
        result.is_err(),
        "Required secret should reject empty string"
    );

    let result = param.validate(&Value::text("password123")).await;
    assert!(
        result.is_ok(),
        "Required secret should accept non-empty string"
    );
}

#[tokio::test]
async fn test_required_code_rejects_empty_string() {
    let param = CodeParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::text("")).await;
    assert!(result.is_err(), "Required code should reject empty string");

    let result = param.validate(&Value::text("fn main() {}")).await;
    assert!(
        result.is_ok(),
        "Required code should accept non-empty string"
    );
}

#[tokio::test]
async fn test_required_list_rejects_empty_array() {
    let param = ListParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::array_empty()).await;
    assert!(result.is_err(), "Required list should reject empty array");

    let result = param.validate(&Value::Null).await;
    assert!(result.is_err(), "Required list should reject Null");

    let non_empty_array = Value::Array(Array::from_vec(vec![Value::integer(1)]));
    let result = param.validate(&non_empty_array).await;
    assert!(
        result.is_ok(),
        "Required list should accept non-empty array"
    );
}

#[tokio::test]
async fn test_required_multi_select_rejects_empty_array() {
    let param = MultiSelectParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    let result = param.validate(&Value::array_empty()).await;
    assert!(
        result.is_err(),
        "Required multi_select should reject empty array"
    );
}

// =============================================================================
// Optional fields - should accept empty values
// =============================================================================

#[tokio::test]
async fn test_optional_text_accepts_empty_string() {
    let param = TextParameter::builder()
        .key("test")
        .name("Test")
        .required(false)
        .build()
        .unwrap();

    let result = param.validate(&Value::text("")).await;
    assert!(result.is_ok(), "Optional text should accept empty string");

    let result = param.validate(&Value::Null).await;
    assert!(result.is_ok(), "Optional text should accept Null");
}

#[tokio::test]
async fn test_optional_list_accepts_empty_array() {
    let param = ListParameter::builder()
        .key("test")
        .name("Test")
        .required(false)
        .build()
        .unwrap();

    let result = param.validate(&Value::array_empty()).await;
    assert!(result.is_ok(), "Optional list should accept empty array");

    let result = param.validate(&Value::Null).await;
    assert!(result.is_ok(), "Optional list should accept Null");
}

// =============================================================================
// Type mismatch - is_empty should not panic on wrong types
// =============================================================================

#[test]
fn test_is_empty_handles_type_mismatch_gracefully() {
    let text_param = TextParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    // Text parameter receiving array - should not panic, is_empty returns false
    // (type validation will catch this separately)
    let _ = text_param.is_empty(&Value::array_empty());
    let _ = text_param.is_empty(&Value::integer(42));

    let list_param = ListParameter::builder()
        .key("test")
        .name("Test")
        .required(true)
        .build()
        .unwrap();

    // List parameter receiving string - should not panic
    let _ = list_param.is_empty(&Value::text("hello"));
    let _ = list_param.is_empty(&Value::integer(42));
}
