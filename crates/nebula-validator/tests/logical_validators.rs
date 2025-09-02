//! Tests for logical validators (AND, OR, NOT, XOR)

use nebula_validator::{
    Validatable, ValidationResult, ValidationError, ErrorCode,
    validators::logical::{And, Or, Not, Xor},
    validators::basic::{AlwaysValid, AlwaysInvalid},
};
use serde_json::Value;
use crate::common::*;

#[tokio::test]
async fn test_and_validator() {
    // Both validators succeed
    let validator = And::new(
        AlwaysValid::new(),
        AlwaysValid::new()
    );
    
    let value = serde_json::json!("test");
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Left validator fails
    let validator = And::new(
        AlwaysInvalid::new("Left failed"),
        AlwaysValid::new()
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure_with_message(&result, "Left failed");
    
    // Right validator fails
    let validator = And::new(
        AlwaysValid::new(),
        AlwaysInvalid::new("Right failed")
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure_with_message(&result, "Right failed");
    
    // Both validators fail
    let validator = And::new(
        AlwaysInvalid::new("Left failed"),
        AlwaysInvalid::new("Right failed")
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure_with_message(&result, "Left failed");
}

#[tokio::test]
async fn test_or_validator() {
    // Both validators succeed
    let validator = Or::new(
        AlwaysValid::new(),
        AlwaysValid::new()
    );
    
    let value = serde_json::json!("test");
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Left validator succeeds, right fails
    let validator = Or::new(
        AlwaysValid::new(),
        AlwaysInvalid::new("Right failed")
    );
    
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Left validator fails, right succeeds
    let validator = Or::new(
        AlwaysInvalid::new("Left failed"),
        AlwaysValid::new()
    );
    
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Both validators fail
    let validator = Or::new(
        AlwaysInvalid::new("Left failed"),
        AlwaysInvalid::new("Right failed")
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure(&result);
    
    // Check that both error messages are included
    let errors = result.errors();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().any(|e| e.message.contains("Left failed")));
    assert!(errors.iter().any(|e| e.message.contains("Right failed")));
}

#[tokio::test]
async fn test_not_validator() {
    // NOT of a failing validator should succeed
    let validator = Not::new(AlwaysInvalid::new("Failed"));
    
    let value = serde_json::json!("test");
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // NOT of a succeeding validator should fail
    let validator = Not::new(AlwaysValid::new());
    
    let result = validator.validate(&value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_xor_validator() {
    // Exactly one validator succeeds
    let validator = Xor::new(
        AlwaysValid::new(),
        AlwaysInvalid::new("Failed")
    );
    
    let value = serde_json::json!("test");
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Both validators succeed (should fail)
    let validator = Xor::new(
        AlwaysValid::new(),
        AlwaysValid::new()
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure(&result);
    
    // Both validators fail (should fail)
    let validator = Xor::new(
        AlwaysInvalid::new("Left failed"),
        AlwaysInvalid::new("Right failed")
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_complex_logical_combinations() {
    // Test complex combinations: (A AND B) OR (C AND D)
    let validator = Or::new(
        And::new(
            AlwaysValid::new(),
            AlwaysValid::new()
        ),
        And::new(
            AlwaysInvalid::new("C failed"),
            AlwaysInvalid::new("D failed")
        )
    );
    
    let value = serde_json::json!("test");
    let result = validator.validate(&value).await;
    assert_validation_success(&result);
    
    // Test: NOT (A OR B)
    let validator = Not::new(
        Or::new(
            AlwaysValid::new(),
            AlwaysInvalid::new("B failed")
        )
    );
    
    let result = validator.validate(&value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_logical_validator_metadata() {
    let and_validator = And::new(AlwaysValid::new(), AlwaysValid::new());
    let metadata = and_validator.metadata();
    
    assert_eq!(metadata.id, "and");
    assert_eq!(metadata.name, "and");
    assert_eq!(metadata.category, ValidatorCategory::Logical);
    
    let or_validator = Or::new(AlwaysValid::new(), AlwaysValid::new());
    let metadata = or_validator.metadata();
    
    assert_eq!(metadata.id, "or");
    assert_eq!(metadata.name, "or");
    assert_eq!(metadata.category, ValidatorCategory::Logical);
    
    let not_validator = Not::new(AlwaysValid::new());
    let metadata = not_validator.metadata();
    
    assert_eq!(metadata.id, "not");
    assert_eq!(metadata.name, "not");
    assert_eq!(metadata.category, ValidatorCategory::Logical);
    
    let xor_validator = Xor::new(AlwaysValid::new(), AlwaysValid::new());
    let metadata = xor_validator.metadata();
    
    assert_eq!(metadata.id, "xor");
    assert_eq!(metadata.name, "xor");
    assert_eq!(metadata.category, ValidatorCategory::Logical);
}

#[tokio::test]
async fn test_logical_validator_complexity() {
    let simple_validator = AlwaysValid::new();
    let complex_validator = AlwaysInvalid::new("test");
    
    let and_validator = And::new(simple_validator.clone(), complex_validator.clone());
    assert_eq!(and_validator.complexity(), ValidationComplexity::Trivial);
    
    let or_validator = Or::new(simple_validator.clone(), complex_validator.clone());
    assert_eq!(or_validator.complexity(), ValidationComplexity::Trivial);
    
    let not_validator = Not::new(simple_validator.clone());
    assert_eq!(not_validator.complexity(), ValidationComplexity::Trivial);
    
    let xor_validator = Xor::new(simple_validator, complex_validator);
    assert_eq!(xor_validator.complexity(), ValidationComplexity::Trivial);
}

#[tokio::test]
async fn test_logical_validator_cacheability() {
    let and_validator = And::new(AlwaysValid::new(), AlwaysValid::new());
    assert!(and_validator.is_cacheable());
    
    let or_validator = Or::new(AlwaysValid::new(), AlwaysValid::new());
    assert!(or_validator.is_cacheable());
    
    let not_validator = Not::new(AlwaysValid::new());
    assert!(not_validator.is_cacheable());
    
    let xor_validator = Xor::new(AlwaysValid::new(), AlwaysValid::new());
    assert!(xor_validator.is_cacheable());
}

#[tokio::test]
async fn test_logical_validator_accepts() {
    let and_validator = And::new(AlwaysValid::new(), AlwaysValid::new());
    let or_validator = Or::new(AlwaysValid::new(), AlwaysValid::new());
    let not_validator = Not::new(AlwaysValid::new());
    let xor_validator = Xor::new(AlwaysValid::new(), AlwaysValid::new());
    
    let test_values = test_json_values();
    for value in test_values {
        assert!(and_validator.accepts(&value));
        assert!(or_validator.accepts(&value));
        assert!(not_validator.accepts(&value));
        assert!(xor_validator.accepts(&value));
    }
}

#[tokio::test]
async fn test_logical_validator_estimate_time() {
    let and_validator = And::new(AlwaysValid::new(), AlwaysValid::new());
    let or_validator = Or::new(AlwaysValid::new(), AlwaysValid::new());
    let not_validator = Not::new(AlwaysValid::new());
    let xor_validator = Xor::new(AlwaysValid::new(), AlwaysValid::new());
    
    let value = serde_json::json!("test");
    
    let and_time = and_validator.estimate_time_ms(&value);
    let or_time = or_validator.estimate_time_ms(&value);
    let not_time = not_validator.estimate_time_ms(&value);
    let xor_time = xor_validator.estimate_time_ms(&value);
    
    assert!(and_time > 0);
    assert!(or_time > 0);
    assert!(not_time > 0);
    assert!(xor_time > 0);
    
    // Logical validators should be fast since they just combine results
    assert!(and_time <= 10);
    assert!(or_time <= 10);
    assert!(not_time <= 5);
    assert!(xor_time <= 10);
}
