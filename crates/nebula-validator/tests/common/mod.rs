//! Common test utilities for nebula-validator

use nebula_validator::{
    Validatable, ValidationResult, ValidationError, ErrorCode,
    ValidatorMetadata, ValidationComplexity, ValidatorCategory,
};
use serde_json::Value;
use async_trait::async_trait;

/// Create a simple test validator that always succeeds
pub fn create_always_valid_validator() -> impl Validatable {
    struct AlwaysValidValidator;
    
    #[async_trait::async_trait]
    impl Validatable for AlwaysValidValidator {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new(
                "always_valid",
                "Always Valid Validator",
                ValidatorCategory::Basic,
            )
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Trivial
        }
    }
    
    AlwaysValidValidator
}

/// Create a simple test validator that always fails
pub fn create_always_invalid_validator(message: &'static str) -> impl Validatable {
    struct AlwaysInvalidValidator {
        message: &'static str,
    }
    
    #[async_trait::async_trait]
    impl Validatable for AlwaysInvalidValidator {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::failure(vec![
                ValidationError::new(ErrorCode::ValidationFailed, self.message)
            ])
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new(
                "always_invalid",
                "Always Invalid Validator",
                ValidatorCategory::Basic,
            )
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Trivial
        }
    }
    
    AlwaysInvalidValidator { message }
}

/// Create a test validator that validates string length
pub fn create_string_length_validator(min: Option<usize>, max: Option<usize>) -> impl Validatable {
    use nebula_validator::validators::string::StringLength;
    StringLength::new(min, max)
}

/// Create a test validator that validates numeric range
pub fn create_numeric_range_validator(min: Option<f64>, max: Option<f64>) -> impl Validatable {
    use nebula_validator::validators::numeric::Numeric;
    Numeric::new(min, max)
}

/// Helper function to create test JSON values
pub fn test_json_values() -> Vec<Value> {
    vec![
        serde_json::json!(null),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(42),
        serde_json::json!(3.14),
        serde_json::json!("hello"),
        serde_json::json!([]),
        serde_json::json!([1, 2, 3]),
        serde_json::json!({}),
        serde_json::json!({"key": "value"}),
    ]
}

/// Helper function to create test strings
pub fn test_strings() -> Vec<Value> {
    vec![
        serde_json::json!(""),
        serde_json::json!("a"),
        serde_json::json!("hello"),
        serde_json::json!("hello world"),
        serde_json::json!("very long string that exceeds normal limits"),
    ]
}

/// Helper function to create test numbers
pub fn test_numbers() -> Vec<Value> {
    vec![
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(42),
        serde_json::json!(-1),
        serde_json::json!(-42),
        serde_json::json!(3.14),
        serde_json::json!(-3.14),
        serde_json::json!(f64::MAX),
        serde_json::json!(f64::MIN),
    ]
}

/// Helper function to create test arrays
pub fn test_arrays() -> Vec<Value> {
    vec![
        serde_json::json!([]),
        serde_json::json!([1]),
        serde_json::json!([1, 2, 3]),
        serde_json::json!(["a", "b", "c"]),
        serde_json::json!([1, "string", true, null]),
    ]
}

/// Helper function to create test objects
pub fn test_objects() -> Vec<Value> {
    vec![
        serde_json::json!({}),
        serde_json::json!({"key": "value"}),
        serde_json::json!({"a": 1, "b": 2, "c": 3}),
        serde_json::json!({"nested": {"key": "value"}}),
    ]
}

/// Assert that a validation result is successful
pub fn assert_validation_success(result: &ValidationResult<()>) {
    assert!(result.is_success(), "Expected validation to succeed, but got: {:?}", result);
}

/// Assert that a validation result failed
pub fn assert_validation_failure(result: &ValidationResult<()>) {
    assert!(result.is_failure(), "Expected validation to fail, but got: {:?}", result);
}

/// Assert that a validation result failed with specific error code
pub fn assert_validation_failure_with_code(result: &ValidationResult<()>, expected_code: ErrorCode) {
    assert_validation_failure(result);
    let errors = result.errors();
    assert!(!errors.is_empty(), "Expected errors but got none");
    assert_eq!(errors[0].code, expected_code, "Expected error code {:?}, but got {:?}", expected_code, errors[0].code);
}

/// Assert that a validation result failed with specific message
pub fn assert_validation_failure_with_message(result: &ValidationResult<()>, expected_message: &str) {
    assert_validation_failure(result);
    let errors = result.errors();
    assert!(!errors.is_empty(), "Expected errors but got none");
    assert!(errors[0].message.contains(expected_message), 
        "Expected error message to contain '{}', but got '{}'", expected_message, errors[0].message);
}
