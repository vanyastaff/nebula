//! Tests for basic validators

use nebula_validator::{
    Validatable, ValidationResult, ValidationError, ErrorCode,
    validators::{AlwaysValid, AlwaysInvalid, Predicate, NotNull},
};
use serde_json::Value;
use crate::common::*;

#[tokio::test]
async fn test_always_valid_validator() {
    let validator = AlwaysValid::new();
    let test_values = test_json_values();
    
    for value in test_values {
        let result = validator.validate(&value).await;
        assert_validation_success(&result);
    }
}

#[tokio::test]
async fn test_always_invalid_validator() {
    let message = "Always fails";
    let validator = AlwaysInvalid::new(message);
    let test_values = test_json_values();
    
    for value in test_values {
        let result = validator.validate(&value).await;
        assert_validation_failure_with_message(&result, message);
    }
}

#[tokio::test]
async fn test_predicate_validator() {
    // Test with a simple predicate that checks if value is a string
    let validator = Predicate::new(|value: &Value| {
        value.is_string()
    });
    
    let string_values = vec![
        serde_json::json!("hello"),
        serde_json::json!(""),
        serde_json::json!("123"),
    ];
    
    let non_string_values = vec![
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!(null),
    ];
    
    // Test string values (should pass)
    for value in string_values {
        let result = validator.validate(&value).await;
        assert_validation_success(&result);
    }
    
    // Test non-string values (should fail)
    for value in non_string_values {
        let result = validator.validate(&value).await;
        assert_validation_failure(&result);
    }
}

#[tokio::test]
async fn test_not_null_validator() {
    let validator = NotNull::new();
    
    // Test null value (should fail)
    let null_value = serde_json::json!(null);
    let result = validator.validate(&null_value).await;
    assert_validation_failure(&result);
    
    // Test non-null values (should pass)
    let non_null_values = vec![
        serde_json::json!("hello"),
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!([]),
        serde_json::json!({}),
    ];
    
    for value in non_null_values {
        let result = validator.validate(&value).await;
        assert_validation_success(&result);
    }
}

#[tokio::test]
async fn test_predicate_with_error_message() {
    let validator = Predicate::new_with_message(
        |value: &Value| value.is_string(),
        "Value must be a string"
    );
    
    let string_value = serde_json::json!("hello");
    let result = validator.validate(&string_value).await;
    assert_validation_success(&result);
    
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure_with_message(&result, "Value must be a string");
}

#[tokio::test]
async fn test_validator_metadata() {
    let validator = AlwaysValid::new();
    let metadata = validator.metadata();
    
    assert_eq!(metadata.id, "always_valid");
    assert_eq!(metadata.name, "Always Valid");
    assert_eq!(metadata.category, ValidatorCategory::Basic);
}

#[tokio::test]
async fn test_validator_complexity() {
    let always_valid = AlwaysValid::new();
    assert_eq!(always_valid.complexity(), ValidationComplexity::Trivial);
    
    let always_invalid = AlwaysInvalid::new("test");
    assert_eq!(always_invalid.complexity(), ValidationComplexity::Trivial);
    
    let not_null = NotNull::new();
    assert_eq!(not_null.complexity(), ValidationComplexity::Simple);
}

#[tokio::test]
async fn test_validator_cacheability() {
    let always_valid = AlwaysValid::new();
    assert!(always_valid.is_cacheable());
    
    let always_invalid = AlwaysInvalid::new("test");
    assert!(always_invalid.is_cacheable());
    
    let not_null = NotNull::new();
    assert!(not_null.is_cacheable());
}

#[tokio::test]
async fn test_cache_key_generation() {
    let validator = AlwaysValid::new();
    let value = serde_json::json!("test");
    
    let cache_key = validator.cache_key(&value);
    assert!(cache_key.is_some());
    
    let key = cache_key.unwrap();
    assert!(key.contains("always_valid"));
    assert!(key.contains("test"));
}

#[tokio::test]
async fn test_validator_accepts() {
    let validator = AlwaysValid::new();
    
    // By default, validators accept all value types
    let test_values = test_json_values();
    for value in test_values {
        assert!(validator.accepts(&value));
    }
}

#[tokio::test]
async fn test_estimate_time() {
    let validator = AlwaysValid::new();
    let value = serde_json::json!("test");
    
    let estimated_time = validator.estimate_time_ms(&value);
    assert!(estimated_time > 0);
    assert!(estimated_time <= 100); // Should be very fast for trivial validators
}
