//! Tests for numeric validators

use nebula_validator::{
    Validatable, ValidationResult, ValidationError, ErrorCode,
    validators::numeric::{Numeric, Positive, Negative, Zero, NonZero, Even, Odd},
};
use serde_json::Value;
use crate::common::*;

#[tokio::test]
async fn test_numeric_range_validator() {
    // Test minimum value
    let validator = Numeric::new(Some(10.0), None);
    
    let small_number = serde_json::json!(5);
    let result = validator.validate(&small_number).await;
    assert_validation_failure(&result);
    
    let valid_number = serde_json::json!(10);
    let result = validator.validate(&valid_number).await;
    assert_validation_success(&result);
    
    let large_number = serde_json::json!(15);
    let result = validator.validate(&large_number).await;
    assert_validation_success(&result);
    
    // Test maximum value
    let validator = Numeric::new(None, Some(20.0));
    
    let small_number = serde_json::json!(5);
    let result = validator.validate(&small_number).await;
    assert_validation_success(&result);
    
    let valid_number = serde_json::json!(20);
    let result = validator.validate(&valid_number).await;
    assert_validation_success(&result);
    
    let large_number = serde_json::json!(25);
    let result = validator.validate(&large_number).await;
    assert_validation_failure(&result);
    
    // Test range
    let validator = Numeric::new(Some(10.0), Some(20.0));
    
    let small_number = serde_json::json!(5);
    let result = validator.validate(&small_number).await;
    assert_validation_failure(&result);
    
    let valid_number = serde_json::json!(15);
    let result = validator.validate(&valid_number).await;
    assert_validation_success(&result);
    
    let large_number = serde_json::json!(25);
    let result = validator.validate(&large_number).await;
    assert_validation_failure(&result);
    
    // Test with floating point numbers
    let validator = Numeric::new(Some(10.5), Some(20.5));
    
    let valid_float = serde_json::json!(15.7);
    let result = validator.validate(&valid_float).await;
    assert_validation_success(&result);
    
    let invalid_float = serde_json::json!(25.3);
    let result = validator.validate(&invalid_float).await;
    assert_validation_failure(&result);
    
    // Test non-numeric values
    let validator = Numeric::new(Some(10.0), Some(20.0));
    
    let string_value = serde_json::json!("15");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
    
    let array_value = serde_json::json!([1, 2, 3]);
    let result = validator.validate(&array_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_positive_validator() {
    let validator = Positive::new();
    
    // Positive numbers should pass
    let positive_numbers = vec![
        serde_json::json!(1),
        serde_json::json!(42),
        serde_json::json!(3.14),
        serde_json::json!(0.001),
    ];
    
    for number in positive_numbers {
        let result = validator.validate(&number).await;
        assert_validation_success(&result);
    }
    
    // Non-positive numbers should fail
    let non_positive_numbers = vec![
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(-42),
        serde_json::json!(-3.14),
    ];
    
    for number in non_positive_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("42");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_negative_validator() {
    let validator = Negative::new();
    
    // Negative numbers should pass
    let negative_numbers = vec![
        serde_json::json!(-1),
        serde_json::json!(-42),
        serde_json::json!(-3.14),
        serde_json::json!(-0.001),
    ];
    
    for number in negative_numbers {
        let result = validator.validate(&number).await;
        assert_validation_success(&result);
    }
    
    // Non-negative numbers should fail
    let non_negative_numbers = vec![
        serde_json::json!(0),
        serde_json::json!(1),
        serde_json::json!(42),
        serde_json::json!(3.14),
    ];
    
    for number in non_negative_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("-42");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_zero_validator() {
    let validator = Zero::new();
    
    // Zero should pass
    let zero_values = vec![
        serde_json::json!(0),
        serde_json::json!(0.0),
    ];
    
    for value in zero_values {
        let result = validator.validate(&value).await;
        assert_validation_success(&result);
    }
    
    // Non-zero values should fail
    let non_zero_values = vec![
        serde_json::json!(1),
        serde_json::json!(-1),
        serde_json::json!(42),
        serde_json::json!(-42),
        serde_json::json!(3.14),
        serde_json::json!(-3.14),
    ];
    
    for value in non_zero_values {
        let result = validator.validate(&value).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("0");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_non_zero_validator() {
    let validator = NonZero::new();
    
    // Non-zero values should pass
    let non_zero_values = vec![
        serde_json::json!(1),
        serde_json::json!(-1),
        serde_json::json!(42),
        serde_json::json!(-42),
        serde_json::json!(3.14),
        serde_json::json!(-3.14),
    ];
    
    for value in non_zero_values {
        let result = validator.validate(&value).await;
        assert_validation_success(&result);
    }
    
    // Zero should fail
    let zero_values = vec![
        serde_json::json!(0),
        serde_json::json!(0.0),
    ];
    
    for value in zero_values {
        let result = validator.validate(&value).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("42");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_even_validator() {
    let validator = Even::new();
    
    // Even numbers should pass
    let even_numbers = vec![
        serde_json::json!(0),
        serde_json::json!(2),
        serde_json::json!(42),
        serde_json::json!(-2),
        serde_json::json!(-42),
    ];
    
    for number in even_numbers {
        let result = validator.validate(&number).await;
        assert_validation_success(&result);
    }
    
    // Odd numbers should fail
    let odd_numbers = vec![
        serde_json::json!(1),
        serde_json::json!(3),
        serde_json::json!(41),
        serde_json::json!(-1),
        serde_json::json!(-3),
    ];
    
    for number in odd_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Floating point numbers should fail (not integers)
    let float_numbers = vec![
        serde_json::json!(2.0),
        serde_json::json!(3.14),
        serde_json::json!(-2.5),
    ];
    
    for number in float_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("42");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_odd_validator() {
    let validator = Odd::new();
    
    // Odd numbers should pass
    let odd_numbers = vec![
        serde_json::json!(1),
        serde_json::json!(3),
        serde_json::json!(41),
        serde_json::json!(-1),
        serde_json::json!(-3),
    ];
    
    for number in odd_numbers {
        let result = validator.validate(&number).await;
        assert_validation_success(&result);
    }
    
    // Even numbers should fail
    let even_numbers = vec![
        serde_json::json!(0),
        serde_json::json!(2),
        serde_json::json!(42),
        serde_json::json!(-2),
        serde_json::json!(-42),
    ];
    
    for number in even_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Floating point numbers should fail (not integers)
    let float_numbers = vec![
        serde_json::json!(1.0),
        serde_json::json!(3.14),
        serde_json::json!(-1.5),
    ];
    
    for number in float_numbers {
        let result = validator.validate(&number).await;
        assert_validation_failure(&result);
    }
    
    // Test non-numeric values
    let string_value = serde_json::json!("41");
    let result = validator.validate(&string_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_numeric_validator_metadata() {
    let range_validator = Numeric::new(Some(10.0), Some(20.0));
    let metadata = range_validator.metadata();
    
    assert_eq!(metadata.id, "numeric");
    assert_eq!(metadata.name, "Numeric");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let positive_validator = Positive::new();
    let metadata = positive_validator.metadata();
    
    assert_eq!(metadata.id, "positive");
    assert_eq!(metadata.name, "Positive");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let negative_validator = Negative::new();
    let metadata = negative_validator.metadata();
    
    assert_eq!(metadata.id, "negative");
    assert_eq!(metadata.name, "Negative");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let zero_validator = Zero::new();
    let metadata = zero_validator.metadata();
    
    assert_eq!(metadata.id, "zero");
    assert_eq!(metadata.name, "Zero");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let non_zero_validator = NonZero::new();
    let metadata = non_zero_validator.metadata();
    
    assert_eq!(metadata.id, "non_zero");
    assert_eq!(metadata.name, "Non-Zero");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let even_validator = Even::new();
    let metadata = even_validator.metadata();
    
    assert_eq!(metadata.id, "even");
    assert_eq!(metadata.name, "Even");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
    
    let odd_validator = Odd::new();
    let metadata = odd_validator.metadata();
    
    assert_eq!(metadata.id, "odd");
    assert_eq!(metadata.name, "Odd");
    assert_eq!(metadata.category, ValidatorCategory::Numeric);
}

#[tokio::test]
async fn test_numeric_validator_complexity() {
    let range_validator = Numeric::new(Some(10.0), Some(20.0));
    assert_eq!(range_validator.complexity(), ValidationComplexity::Simple);
    
    let positive_validator = Positive::new();
    assert_eq!(positive_validator.complexity(), ValidationComplexity::Simple);
    
    let negative_validator = Negative::new();
    assert_eq!(negative_validator.complexity(), ValidationComplexity::Simple);
    
    let zero_validator = Zero::new();
    assert_eq!(zero_validator.complexity(), ValidationComplexity::Simple);
    
    let non_zero_validator = NonZero::new();
    assert_eq!(non_zero_validator.complexity(), ValidationComplexity::Simple);
    
    let even_validator = Even::new();
    assert_eq!(even_validator.complexity(), ValidationComplexity::Simple);
    
    let odd_validator = Odd::new();
    assert_eq!(odd_validator.complexity(), ValidationComplexity::Simple);
}

#[tokio::test]
async fn test_numeric_validator_accepts() {
    let range_validator = Numeric::new(Some(10.0), Some(20.0));
    let positive_validator = Positive::new();
    let negative_validator = Negative::new();
    let zero_validator = Zero::new();
    let non_zero_validator = NonZero::new();
    let even_validator = Even::new();
    let odd_validator = Odd::new();
    
    // Numeric validators should only accept numeric values
    let number_value = serde_json::json!(15);
    let string_value = serde_json::json!("15");
    
    assert!(range_validator.accepts(&number_value));
    assert!(!range_validator.accepts(&string_value));
    
    assert!(positive_validator.accepts(&number_value));
    assert!(!positive_validator.accepts(&string_value));
    
    assert!(negative_validator.accepts(&number_value));
    assert!(!negative_validator.accepts(&string_value));
    
    assert!(zero_validator.accepts(&number_value));
    assert!(!zero_validator.accepts(&string_value));
    
    assert!(non_zero_validator.accepts(&number_value));
    assert!(!non_zero_validator.accepts(&string_value));
    
    assert!(even_validator.accepts(&number_value));
    assert!(!even_validator.accepts(&string_value));
    
    assert!(odd_validator.accepts(&number_value));
    assert!(!odd_validator.accepts(&string_value));
}

#[tokio::test]
async fn test_numeric_validator_estimate_time() {
    let range_validator = Numeric::new(Some(10.0), Some(20.0));
    let positive_validator = Positive::new();
    let negative_validator = Negative::new();
    let zero_validator = Zero::new();
    let non_zero_validator = NonZero::new();
    let even_validator = Even::new();
    let odd_validator = Odd::new();
    
    let small_number = serde_json::json!(15);
    let large_number = serde_json::json!(1000000);
    
    // Test small numbers
    let range_time_small = range_validator.estimate_time_ms(&small_number);
    let positive_time_small = positive_validator.estimate_time_ms(&small_number);
    let negative_time_small = negative_validator.estimate_time_ms(&small_number);
    let zero_time_small = zero_validator.estimate_time_ms(&small_number);
    let non_zero_time_small = non_zero_validator.estimate_time_ms(&small_number);
    let even_time_small = even_validator.estimate_time_ms(&small_number);
    let odd_time_small = odd_validator.estimate_time_ms(&small_number);
    
    assert!(range_time_small > 0);
    assert!(positive_time_small > 0);
    assert!(negative_time_small > 0);
    assert!(zero_time_small > 0);
    assert!(non_zero_time_small > 0);
    assert!(even_time_small > 0);
    assert!(odd_time_small > 0);
    
    // Test large numbers
    let range_time_large = range_validator.estimate_time_ms(&large_number);
    let positive_time_large = positive_validator.estimate_time_ms(&large_number);
    let negative_time_large = negative_validator.estimate_time_ms(&large_number);
    let zero_time_large = zero_validator.estimate_time_ms(&large_number);
    let non_zero_time_large = non_zero_validator.estimate_time_ms(&large_number);
    let even_time_large = even_validator.estimate_time_ms(&large_number);
    let odd_time_large = odd_validator.estimate_time_ms(&large_number);
    
    assert!(range_time_large > 0);
    assert!(positive_time_large > 0);
    assert!(negative_time_large > 0);
    assert!(zero_time_large > 0);
    assert!(non_zero_time_large > 0);
    assert!(even_time_large > 0);
    assert!(odd_time_large > 0);
    
    // Numeric validators should be very fast regardless of number size
    assert!(range_time_small <= 5);
    assert!(positive_time_small <= 5);
    assert!(negative_time_small <= 5);
    assert!(zero_time_small <= 5);
    assert!(non_zero_time_small <= 5);
    assert!(even_time_small <= 5);
    assert!(odd_time_small <= 5);
}
