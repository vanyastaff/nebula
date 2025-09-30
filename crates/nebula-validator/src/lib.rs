//! Nebula Validator - Production-ready validation framework with advanced combinators and cross-field validation

pub mod core;         // Core validation types and systems
pub mod validators;   // Validators (concrete implementations)


// Re-export core types and traits
pub use core::{
    // Core types
    Valid, Invalid, ValidationError, ValidatorId,
    CoreError, CoreResult,
    // Main traits
    Validator, ValidatorExt, ValidationContext, ValidationComplexity,
    // Logical combinators
    AndValidator, OrValidator, NotValidator, ConditionalValidator,
    // Builder patterns
    ValidationBuilder, BuiltValidator, validate,
    // Value extensions
    ValueExt,
};

// Re-export validators
pub use validators::{
    basic::*,
    cross_field::*,
    string::*,
    numeric::*,
    collection::*,
    comparison::*,
    patterns::*,
    sets::*,
    types::*,
    structural::*,
    dimensions::*,
    files::*,
};

// Re-export common dependencies
pub use nebula_value::Value;
pub use async_trait::async_trait;

// ==================== CONVENIENCE BUILDER API ====================

/// Create a string validator with length constraints using builder pattern
#[bon::builder]
pub fn string_constraints(
    min_len: Option<usize>,
    max_len: Option<usize>,
    #[builder(default = false)]
    alphanumeric_only: bool,
    #[builder(default = false)]
    allow_spaces: bool
) -> Box<dyn Validator> {
    // Build validator chain by conditionally chaining validators
    match (min_len, max_len, alphanumeric_only) {
        (Some(min_val), Some(max_val), true) => {
            Box::new(string()
                .and(min_length(min_val))
                .and(max_length(max_val))
                .and(alphanumeric(allow_spaces)))
        },
        (Some(min_val), Some(max_val), false) => {
            Box::new(string()
                .and(min_length(min_val))
                .and(max_length(max_val)))
        },
        (Some(min_val), None, true) => {
            Box::new(string()
                .and(min_length(min_val))
                .and(alphanumeric(allow_spaces)))
        },
        (Some(min_val), None, false) => {
            Box::new(string().and(min_length(min_val)))
        },
        (None, Some(max_val), true) => {
            Box::new(string()
                .and(max_length(max_val))
                .and(alphanumeric(allow_spaces)))
        },
        (None, Some(max_val), false) => {
            Box::new(string().and(max_length(max_val)))
        },
        (None, None, true) => {
            Box::new(string().and(alphanumeric(allow_spaces)))
        },
        (None, None, false) => {
            Box::new(string())
        },
    }
}

/// Create a numeric validator with range constraints using builder pattern
#[bon::builder]
pub fn number_constraints(
    min_val: Option<f64>,
    max_val: Option<f64>,
    #[builder(default = false)]
    integer_only: bool,
    #[builder(default = false)]
    positive_only: bool
) -> Box<dyn Validator> {
    // Build validator chain by conditionally chaining validators
    match (min_val, max_val, integer_only, positive_only) {
        (Some(min_v), Some(max_v), true, true) => {
            Box::new(number()
                .and(min(min_v))
                .and(max(max_v))
                .and(integer())
                .and(positive()))
        },
        (Some(min_v), Some(max_v), true, false) => {
            Box::new(number()
                .and(min(min_v))
                .and(max(max_v))
                .and(integer()))
        },
        (Some(min_v), Some(max_v), false, true) => {
            Box::new(number()
                .and(min(min_v))
                .and(max(max_v))
                .and(positive()))
        },
        (Some(min_v), Some(max_v), false, false) => {
            Box::new(number()
                .and(min(min_v))
                .and(max(max_v)))
        },
        (Some(min_v), None, true, true) => {
            Box::new(number()
                .and(min(min_v))
                .and(integer())
                .and(positive()))
        },
        (Some(min_v), None, true, false) => {
            Box::new(number()
                .and(min(min_v))
                .and(integer()))
        },
        (Some(min_v), None, false, true) => {
            Box::new(number()
                .and(min(min_v))
                .and(positive()))
        },
        (Some(min_v), None, false, false) => {
            Box::new(number().and(min(min_v)))
        },
        (None, Some(max_v), true, true) => {
            Box::new(number()
                .and(max(max_v))
                .and(integer())
                .and(positive()))
        },
        (None, Some(max_v), true, false) => {
            Box::new(number()
                .and(max(max_v))
                .and(integer()))
        },
        (None, Some(max_v), false, true) => {
            Box::new(number()
                .and(max(max_v))
                .and(positive()))
        },
        (None, Some(max_v), false, false) => {
            Box::new(number().and(max(max_v)))
        },
        (None, None, true, true) => {
            Box::new(number()
                .and(integer())
                .and(positive()))
        },
        (None, None, true, false) => {
            Box::new(number().and(integer()))
        },
        (None, None, false, true) => {
            Box::new(number().and(positive()))
        },
        (None, None, false, false) => {
            Box::new(number())
        },
    }
}

// ==================== ADDITIONAL BUILDER CONVENIENCES ====================

/// Create a collection validator with size constraints using builder pattern
#[bon::builder]
pub fn collection_constraints(
    min_size: Option<usize>,
    max_size: Option<usize>,
    exact_size: Option<usize>,
) -> Box<dyn Validator> {
    match (min_size, max_size, exact_size) {
        (_, _, Some(size)) => Box::new(array_size(size)),
        (Some(min_val), Some(max_val), None) => {
            Box::new(array_min_size(min_val).and(array_max_size(max_val)))
        },
        (Some(min_val), None, None) => Box::new(array_min_size(min_val)),
        (None, Some(max_val), None) => Box::new(array_max_size(max_val)),
        (None, None, None) => Box::new(array()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_value::Value;
    use serde_json::json;

    #[tokio::test]
    async fn test_basic_validation() {
        let validator = required();
        let value = Value::from("hello");
        let result = validator.validate(&value, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_required_validation_fails() {
        let validator = required();
        let value = Value::null();
        let result = validator.validate(&value, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cross_field_validation() {
        // Build password confirmation using composition instead of specialized method
        let validator = equals_field_str("password");
        let root = Value::from(json!({
            "password": "secret123",
            "password_confirmation": "secret123"
        }));
        let context = ValidationContext::simple(root.clone());
        let password_conf_value = Value::from("secret123");

        let result = validator.validate(&password_conf_value, Some(&context)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_logical_combinators() {
        let validator = required().and(not_null());
        let value = Value::from("valid_value");
        let result = validator.validate(&value, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_string_validators() {
        // Min length
        let validator = min_length(5);
        let value = Value::from("hello world");
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from("hi");
        assert!(validator.validate(&value, None).await.is_err());

        // Basic email-like pattern validation using contains
        let email_validator = string_contains("@".to_string())
            .and(string_contains(".".to_string()));
        let value = Value::from("test@example.com");
        assert!(email_validator.validate(&value, None).await.is_ok());

        let value = Value::from("invalid-email");
        assert!(email_validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_numeric_validators() {
        // Min value
        let validator = min(10.0);
        let value = Value::from(15.5);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from(5.0);
        assert!(validator.validate(&value, None).await.is_err());

        // Range
        let validator = range(0.0, 100.0);
        let value = Value::from(50.0);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from(150.0);
        assert!(validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_collection_validators() {
        // Array size
        let validator = size(3);
        let value = Value::from(json!([1, 2, 3]));
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from(json!([1, 2]));
        assert!(validator.validate(&value, None).await.is_err());

        // Non-empty
        let validator = not_empty();
        let value = Value::from(json!([1, 2, 3]));
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from(json!([]));
        assert!(validator.validate(&value, None).await.is_err());

        // Contains
        let validator = array_contains(Value::from("apple"));
        let value = Value::from(json!(["apple", "banana", "orange"]));
        assert!(validator.validate(&value, None).await.is_ok());

        let value = Value::from(json!(["banana", "orange"]));
        assert!(validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_comparison_validators() {
        // Equals
        let validator = equals(Value::from("test"));
        assert!(validator.validate(&Value::from("test"), None).await.is_ok());
        assert!(validator.validate(&Value::from("other"), None).await.is_err());

        // Greater than
        let validator = greater_than(10.0);
        assert!(validator.validate(&Value::from(15), None).await.is_ok());
        assert!(validator.validate(&Value::from(5), None).await.is_err());

        // Between
        let validator = between(0.0, 100.0);
        assert!(validator.validate(&Value::from(50), None).await.is_ok());
        assert!(validator.validate(&Value::from(150), None).await.is_err());
    }

    #[tokio::test]
    async fn test_pattern_validators() {
        // Starts with
        let validator = string_starts_with("hello".to_string());
        assert!(validator.validate(&Value::from("hello world"), None).await.is_ok());
        assert!(validator.validate(&Value::from("world hello"), None).await.is_err());

        // Contains substring
        let validator = string_contains("test".to_string());
        assert!(validator.validate(&Value::from("this is a test"), None).await.is_ok());
        assert!(validator.validate(&Value::from("no match here"), None).await.is_err());

        // Ends with
        let validator = string_ends_with(".com".to_string());
        assert!(validator.validate(&Value::from("site.com"), None).await.is_ok());
        assert!(validator.validate(&Value::from("site.org"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_set_validators() {
        // In set
        let validator = in_str_values(vec!["apple", "banana", "orange"]);
        assert!(validator.validate(&Value::from("apple"), None).await.is_ok());
        assert!(validator.validate(&Value::from("grape"), None).await.is_err());

        // Not in set
        let validator = not_in_str_values(vec!["forbidden", "blocked"]);
        assert!(validator.validate(&Value::from("allowed"), None).await.is_ok());
        assert!(validator.validate(&Value::from("forbidden"), None).await.is_err());

        // One of (string-specific)
        let validator = one_of(vec![Value::from("red"), Value::from("green"), Value::from("blue")]);
        assert!(validator.validate(&Value::from("red"), None).await.is_ok());
        assert!(validator.validate(&Value::from("yellow"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_compositional_validation() {
        // Build complex validation through composition
        // Password must be: required, min 8 chars, contain "y", contain "s", not be common passwords
        let validator = required()
            .and(min_length(8))
            .and(string_contains("y".to_string())) // Contains y
            .and(string_contains("s".to_string()))    // Contains s
            .and(not_in_str_values(vec!["password", "123456", "qwerty"])); // Not common passwords

        let strong_password = Value::from("MyPass123");
        assert!(validator.validate(&strong_password, None).await.is_ok());

        let weak_password = Value::from("password");
        assert!(validator.validate(&weak_password, None).await.is_err());

        let short_password = Value::from("Aa1");
        assert!(validator.validate(&short_password, None).await.is_err());
    }

    #[tokio::test]
    async fn test_email_domain_validation() {
        // Email validation through composition: must match email pattern AND from allowed domains
        let validator = string_contains("@".to_string())
            .and(string_ends_with("@company.com".to_string()).or(string_ends_with("@partner.org".to_string())));

        let valid_email = Value::from("user@company.com");
        assert!(validator.validate(&valid_email, None).await.is_ok());

        let invalid_domain = Value::from("user@external.com");
        assert!(validator.validate(&invalid_domain, None).await.is_err());
    }

    #[tokio::test]
    async fn test_age_validation() {
        // Age validation: must be number, between 0-120, and not exactly forbidden ages
        let validator = greater_than_or_equal(0.0)
            .and(less_than_or_equal(120.0))
            .and(not_equals(Value::from(13))); // Unlucky age example

        assert!(validator.validate(&Value::from(25), None).await.is_ok());
        assert!(validator.validate(&Value::from(13), None).await.is_err());
        assert!(validator.validate(&Value::from(150), None).await.is_err());
    }

    #[tokio::test]
    async fn test_type_validators() {
        // String type
        let validator = string();
        assert!(validator.validate(&Value::from("hello"), None).await.is_ok());
        assert!(validator.validate(&Value::from(123), None).await.is_err());

        // Number type
        let validator = number();
        assert!(validator.validate(&Value::from(42), None).await.is_ok());
        assert!(validator.validate(&Value::from("hello"), None).await.is_err());

        // Boolean type
        let validator = boolean();
        assert!(validator.validate(&Value::from(true), None).await.is_ok());
        assert!(validator.validate(&Value::from(123), None).await.is_err());

        // Array type
        let validator = array();
        assert!(validator.validate(&Value::from(json!([1, 2, 3])), None).await.is_ok());
        assert!(validator.validate(&Value::from(json!({})), None).await.is_err());

        // Object type
        let validator = object();
        assert!(validator.validate(&Value::from(json!({"key": "value"})), None).await.is_ok());
        assert!(validator.validate(&Value::from(json!([])), None).await.is_err());
    }

    #[tokio::test]
    async fn test_advanced_string_validators() {
        // Alphanumeric
        let validator = alphanumeric(false);
        assert!(validator.validate(&Value::from("abc123"), None).await.is_ok());
        assert!(validator.validate(&Value::from("hello@world"), None).await.is_err());

        // Alpha only
        let validator = alpha(false);
        assert!(validator.validate(&Value::from("hello"), None).await.is_ok());
        assert!(validator.validate(&Value::from("hello123"), None).await.is_err());

        // Numeric string
        let validator = numeric_string(false, false);
        assert!(validator.validate(&Value::from("12345"), None).await.is_ok());
        assert!(validator.validate(&Value::from("123.45"), None).await.is_err());

        // Decimal string
        let validator = decimal_string();
        assert!(validator.validate(&Value::from("123.45"), None).await.is_ok());
        assert!(validator.validate(&Value::from("123.45.67"), None).await.is_err());

        // Uppercase
        let validator = uppercase();
        assert!(validator.validate(&Value::from("HELLO"), None).await.is_ok());
        assert!(validator.validate(&Value::from("Hello"), None).await.is_err());

        // Lowercase
        let validator = lowercase();
        assert!(validator.validate(&Value::from("hello"), None).await.is_ok());
        assert!(validator.validate(&Value::from("Hello"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_structural_validators() {
        // Has key
        let validator = has_key("name".to_string());
        let obj = Value::from(json!({"name": "Alice", "age": 30}));
        assert!(validator.validate(&obj, None).await.is_ok());

        let obj_missing = Value::from(json!({"age": 30}));
        assert!(validator.validate(&obj_missing, None).await.is_err());

        // Has all keys
        let validator = has_all_keys(vec!["name".to_string(), "age".to_string()]);
        let complete_obj = Value::from(json!({"name": "Alice", "age": 30}));
        assert!(validator.validate(&complete_obj, None).await.is_ok());

        let incomplete_obj = Value::from(json!({"name": "Alice"}));
        assert!(validator.validate(&incomplete_obj, None).await.is_err());

        // Array contains value
        let validator = array_contains(Value::from("apple"));
        let fruits = Value::from(json!(["apple", "banana", "orange"]));
        assert!(validator.validate(&fruits, None).await.is_ok());

        let no_apple = Value::from(json!(["banana", "orange"]));
        assert!(validator.validate(&no_apple, None).await.is_err());
    }

    #[tokio::test]
    async fn test_advanced_composition() {
        // User profile validation: object with required fields, specific types, and constraints
        let user_validator = object()
            .and(has_all_keys(vec!["username".to_string(), "email".to_string(), "age".to_string()]));

        let user_obj = Value::from(json!({
            "username": "alice123",
            "email": "alice@example.com",
            "age": 25
        }));

        assert!(user_validator.validate(&user_obj, None).await.is_ok());

        // Complex string validation: username must be alphanumeric, 3-20 chars, lowercase
        let username_validator = string()
            .and(min_length(3))
            .and(max_length(20))
            .and(alphanumeric(false))
            .and(lowercase());

        assert!(username_validator.validate(&Value::from("alice123"), None).await.is_ok());
        assert!(username_validator.validate(&Value::from("Alice123"), None).await.is_err()); // Not lowercase
        assert!(username_validator.validate(&Value::from("a!"), None).await.is_err()); // Too short + special char

        // Array of valid items
        let numbers_validator = array()
            .and(min_size(1))
            .and(max_size(10));

        let valid_numbers = Value::from(json!([1, 2, 3, 4, 5]));
        assert!(numbers_validator.validate(&valid_numbers, None).await.is_ok());

        let empty_array = Value::from(json!([]));
        assert!(numbers_validator.validate(&empty_array, None).await.is_err());
    }

    #[tokio::test]
    async fn test_dimension_validators() {
        // Divisible by
        let divisible_validator = divisible_by(3.0);
        assert!(divisible_validator.validate(&Value::from(9), None).await.is_ok());
        assert!(divisible_validator.validate(&Value::from(10), None).await.is_err());

        // Even/Odd
        let even_validator = even();
        assert!(even_validator.validate(&Value::from(4), None).await.is_ok());
        assert!(even_validator.validate(&Value::from(5), None).await.is_err());

        let odd_validator = odd();
        assert!(odd_validator.validate(&Value::from(5), None).await.is_ok());
        assert!(odd_validator.validate(&Value::from(4), None).await.is_err());
    }

    #[tokio::test]
    async fn test_file_validators() {
        // MIME type
        let image_mime = mime_types(vec!["image/jpeg", "image/png"]);
        assert!(image_mime.validate(&Value::from("image/jpeg"), None).await.is_ok());
        assert!(image_mime.validate(&Value::from("text/plain"), None).await.is_err());

        // File extension
        let image_ext = file_extensions(vec!["jpg", "png", "gif"]);
        assert!(image_ext.validate(&Value::from("photo.jpg"), None).await.is_ok());
        assert!(image_ext.validate(&Value::from("document.pdf"), None).await.is_err());

        // File size
        let size_validator = file_size_range(100, 5000);
        assert!(size_validator.validate(&Value::from(2500), None).await.is_ok());
        assert!(size_validator.validate(&Value::from(50), None).await.is_err());
        assert!(size_validator.validate(&Value::from(10000), None).await.is_err());
    }

    #[tokio::test]
    async fn test_collection_enhancements() {
        // Distinct values
        let distinct_validator = unique();
        let unique_array = Value::from(json!([1, 2, 3, 4, 5]));
        assert!(distinct_validator.validate(&unique_array, None).await.is_ok());

        let duplicate_array = Value::from(json!([1, 2, 3, 2, 5]));
        assert!(distinct_validator.validate(&duplicate_array, None).await.is_err());

        // Simplified element validation - test individual validators
        let positive_validator = positive();
        let positive_number = Value::from(5);
        assert!(positive_validator.validate(&positive_number, None).await.is_ok());

        let negative_number = Value::from(-3);
        assert!(positive_validator.validate(&negative_number, None).await.is_err());

        // Negative element validation
        let negative_validator = negative();
        assert!(negative_validator.validate(&negative_number, None).await.is_ok());
        assert!(negative_validator.validate(&positive_number, None).await.is_err());
    }

    #[tokio::test]
    async fn test_builder_patterns() {
        // Simple validation builder
        let validator = validate(string())
            .and(min_length(3))
            .and(alphanumeric(false))
            .named("username_validator")
            .build();

        assert!(validator.validate(&Value::from("user123"), None).await.is_ok());
        assert!(validator.validate(&Value::from("hi"), None).await.is_err());
        assert_eq!(validator.name(), "username_validator");

        // Test AND logic
        let and_validator = string().and(min_length(5));
        assert!(and_validator.validate(&Value::from("hello"), None).await.is_ok());
        assert!(and_validator.validate(&Value::from("hi"), None).await.is_err());
        assert!(and_validator.validate(&Value::from(123), None).await.is_err());

        // Test OR logic
        let or_validator = string().or(number());
        assert!(or_validator.validate(&Value::from("hello"), None).await.is_ok());
        assert!(or_validator.validate(&Value::from(123), None).await.is_ok());
        assert!(or_validator.validate(&Value::from(true), None).await.is_err());

        // Test NOT logic
        let not_validator = string().not();
        assert!(not_validator.validate(&Value::from(123), None).await.is_ok());
        assert!(not_validator.validate(&Value::from("hello"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_bon_builder_api() {
        // Test string validator builder
        let validator = string_constraints()
            .min_len(3)
            .max_len(10)
            .alphanumeric_only(true)
            .allow_spaces(false)
            .call();

        assert!(validator.validate(&Value::from("abc123"), None).await.is_ok());
        assert!(validator.validate(&Value::from("ab"), None).await.is_err()); // Too short
        assert!(validator.validate(&Value::from("abcdefghijk"), None).await.is_err()); // Too long
        assert!(validator.validate(&Value::from("abc@123"), None).await.is_err()); // Not alphanumeric

        // Test number validator builder
        let validator = number_constraints()
            .min_val(0.0)
            .max_val(100.0)
            .positive_only(true)
            .call();

        assert!(validator.validate(&Value::from(50), None).await.is_ok());
        assert!(validator.validate(&Value::from(-5), None).await.is_err()); // Negative
        assert!(validator.validate(&Value::from(150), None).await.is_err()); // Too high

        // Test builder functions from specific modules
        let numeric_validator = numeric_string_builder()
            .allow_decimal(true)
            .allow_negative(false)
            .call();

        assert!(numeric_validator.validate(&Value::from("123.45"), None).await.is_ok());
        assert!(numeric_validator.validate(&Value::from("-123"), None).await.is_err()); // Negative not allowed

        let alpha_validator = alpha_builder()
            .allow_spaces(true)
            .call();

        assert!(alpha_validator.validate(&Value::from("hello world"), None).await.is_ok());
        assert!(alpha_validator.validate(&Value::from("hello123"), None).await.is_err()); // Contains numbers

        // Test ValidationBuilder with manual building
        let builder_validator = validate(string())
            .named("test_string")
            .build();

        assert_eq!(builder_validator.name(), "test_string");
        assert!(builder_validator.validate(&Value::from("hello"), None).await.is_ok());

        // Test collection constraints
        let collection_validator = collection_constraints()
            .min_size(2)
            .max_size(5)
            .call();

        let valid_array = Value::from(json!([1, 2, 3]));
        let too_small = Value::from(json!([1]));
        let too_large = Value::from(json!([1, 2, 3, 4, 5, 6]));

        assert!(collection_validator.validate(&valid_array, None).await.is_ok());
        assert!(collection_validator.validate(&too_small, None).await.is_err());
        assert!(collection_validator.validate(&too_large, None).await.is_err());
    }
}