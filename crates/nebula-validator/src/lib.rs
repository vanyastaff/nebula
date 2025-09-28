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
pub use serde_json::Value;
pub use async_trait::async_trait;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_basic_validation() {
        let validator = required();
        let value = json!("hello");
        let result = validator.validate(&value, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_required_validation_fails() {
        let validator = required();
        let value = json!(null);
        let result = validator.validate(&value, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cross_field_validation() {
        // Build password confirmation using composition instead of specialized method
        let validator = equals_field_str("password");
        let root = json!({
            "password": "secret123",
            "password_confirmation": "secret123"
        });
        let context = ValidationContext::simple(root.clone());
        let password_conf_value = json!("secret123");

        let result = validator.validate(&password_conf_value, Some(&context)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_logical_combinators() {
        let validator = required().and(not_null());
        let value = json!("valid_value");
        let result = validator.validate(&value, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_string_validators() {
        // Min length
        let validator = min_length(5);
        let value = json!("hello world");
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!("hi");
        assert!(validator.validate(&value, None).await.is_err());

        // Basic email-like pattern validation using contains
        let email_validator = string_contains("@".to_string())
            .and(string_contains(".".to_string()));
        let value = json!("test@example.com");
        assert!(email_validator.validate(&value, None).await.is_ok());

        let value = json!("invalid-email");
        assert!(email_validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_numeric_validators() {
        // Min value
        let validator = min(10.0);
        let value = json!(15.5);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!(5.0);
        assert!(validator.validate(&value, None).await.is_err());

        // Range
        let validator = range(0.0, 100.0);
        let value = json!(50.0);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!(150.0);
        assert!(validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_collection_validators() {
        // Array size
        let validator = size(3);
        let value = json!([1, 2, 3]);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!([1, 2]);
        assert!(validator.validate(&value, None).await.is_err());

        // Non-empty
        let validator = not_empty();
        let value = json!([1, 2, 3]);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!([]);
        assert!(validator.validate(&value, None).await.is_err());

        // Contains
        let validator = array_contains(json!("apple"));
        let value = json!(["apple", "banana", "orange"]);
        assert!(validator.validate(&value, None).await.is_ok());

        let value = json!(["banana", "orange"]);
        assert!(validator.validate(&value, None).await.is_err());
    }

    #[tokio::test]
    async fn test_comparison_validators() {
        // Equals
        let validator = equals(json!("test"));
        assert!(validator.validate(&json!("test"), None).await.is_ok());
        assert!(validator.validate(&json!("other"), None).await.is_err());

        // Greater than
        let validator = greater_than(10.0);
        assert!(validator.validate(&json!(15), None).await.is_ok());
        assert!(validator.validate(&json!(5), None).await.is_err());

        // Between
        let validator = between(0.0, 100.0);
        assert!(validator.validate(&json!(50), None).await.is_ok());
        assert!(validator.validate(&json!(150), None).await.is_err());
    }

    #[tokio::test]
    async fn test_pattern_validators() {
        // Starts with
        let validator = string_starts_with("hello".to_string());
        assert!(validator.validate(&json!("hello world"), None).await.is_ok());
        assert!(validator.validate(&json!("world hello"), None).await.is_err());

        // Contains substring
        let validator = string_contains("test".to_string());
        assert!(validator.validate(&json!("this is a test"), None).await.is_ok());
        assert!(validator.validate(&json!("no match here"), None).await.is_err());

        // Ends with
        let validator = string_ends_with(".com".to_string());
        assert!(validator.validate(&json!("site.com"), None).await.is_ok());
        assert!(validator.validate(&json!("site.org"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_set_validators() {
        // In set
        let validator = in_str_values(vec!["apple", "banana", "orange"]);
        assert!(validator.validate(&json!("apple"), None).await.is_ok());
        assert!(validator.validate(&json!("grape"), None).await.is_err());

        // Not in set
        let validator = not_in_str_values(vec!["forbidden", "blocked"]);
        assert!(validator.validate(&json!("allowed"), None).await.is_ok());
        assert!(validator.validate(&json!("forbidden"), None).await.is_err());

        // One of (string-specific)
        let validator = one_of(vec![json!("red"), json!("green"), json!("blue")]);
        assert!(validator.validate(&json!("red"), None).await.is_ok());
        assert!(validator.validate(&json!("yellow"), None).await.is_err());
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

        let strong_password = json!("MyPass123");
        assert!(validator.validate(&strong_password, None).await.is_ok());

        let weak_password = json!("password");
        assert!(validator.validate(&weak_password, None).await.is_err());

        let short_password = json!("Aa1");
        assert!(validator.validate(&short_password, None).await.is_err());
    }

    #[tokio::test]
    async fn test_email_domain_validation() {
        // Email validation through composition: must match email pattern AND from allowed domains
        let validator = string_contains("@".to_string())
            .and(string_ends_with("@company.com".to_string()).or(string_ends_with("@partner.org".to_string())));

        let valid_email = json!("user@company.com");
        assert!(validator.validate(&valid_email, None).await.is_ok());

        let invalid_domain = json!("user@external.com");
        assert!(validator.validate(&invalid_domain, None).await.is_err());
    }

    #[tokio::test]
    async fn test_age_validation() {
        // Age validation: must be number, between 0-120, and not exactly forbidden ages
        let validator = greater_than_or_equal(0.0)
            .and(less_than_or_equal(120.0))
            .and(not_equals(json!(13))); // Unlucky age example

        assert!(validator.validate(&json!(25), None).await.is_ok());
        assert!(validator.validate(&json!(13), None).await.is_err());
        assert!(validator.validate(&json!(150), None).await.is_err());
    }

    #[tokio::test]
    async fn test_type_validators() {
        // String type
        let validator = string();
        assert!(validator.validate(&json!("hello"), None).await.is_ok());
        assert!(validator.validate(&json!(123), None).await.is_err());

        // Number type
        let validator = number();
        assert!(validator.validate(&json!(42), None).await.is_ok());
        assert!(validator.validate(&json!("hello"), None).await.is_err());

        // Boolean type
        let validator = boolean();
        assert!(validator.validate(&json!(true), None).await.is_ok());
        assert!(validator.validate(&json!(123), None).await.is_err());

        // Array type
        let validator = array();
        assert!(validator.validate(&json!([1, 2, 3]), None).await.is_ok());
        assert!(validator.validate(&json!({}), None).await.is_err());

        // Object type
        let validator = object();
        assert!(validator.validate(&json!({"key": "value"}), None).await.is_ok());
        assert!(validator.validate(&json!([]), None).await.is_err());
    }

    #[tokio::test]
    async fn test_advanced_string_validators() {
        // Alphanumeric
        let validator = alphanumeric(false);
        assert!(validator.validate(&json!("abc123"), None).await.is_ok());
        assert!(validator.validate(&json!("hello@world"), None).await.is_err());

        // Alpha only
        let validator = alpha(false);
        assert!(validator.validate(&json!("hello"), None).await.is_ok());
        assert!(validator.validate(&json!("hello123"), None).await.is_err());

        // Numeric string
        let validator = numeric_string(false, false);
        assert!(validator.validate(&json!("12345"), None).await.is_ok());
        assert!(validator.validate(&json!("123.45"), None).await.is_err());

        // Decimal string
        let validator = decimal_string();
        assert!(validator.validate(&json!("123.45"), None).await.is_ok());
        assert!(validator.validate(&json!("123.45.67"), None).await.is_err());

        // Uppercase
        let validator = uppercase();
        assert!(validator.validate(&json!("HELLO"), None).await.is_ok());
        assert!(validator.validate(&json!("Hello"), None).await.is_err());

        // Lowercase
        let validator = lowercase();
        assert!(validator.validate(&json!("hello"), None).await.is_ok());
        assert!(validator.validate(&json!("Hello"), None).await.is_err());
    }

    #[tokio::test]
    async fn test_structural_validators() {
        // Has key
        let validator = has_key("name".to_string());
        let obj = json!({"name": "Alice", "age": 30});
        assert!(validator.validate(&obj, None).await.is_ok());

        let obj_missing = json!({"age": 30});
        assert!(validator.validate(&obj_missing, None).await.is_err());

        // Has all keys
        let validator = has_all_keys(vec!["name".to_string(), "age".to_string()]);
        let complete_obj = json!({"name": "Alice", "age": 30});
        assert!(validator.validate(&complete_obj, None).await.is_ok());

        let incomplete_obj = json!({"name": "Alice"});
        assert!(validator.validate(&incomplete_obj, None).await.is_err());

        // Array contains value
        let validator = array_contains(json!("apple"));
        let fruits = json!(["apple", "banana", "orange"]);
        assert!(validator.validate(&fruits, None).await.is_ok());

        let no_apple = json!(["banana", "orange"]);
        assert!(validator.validate(&no_apple, None).await.is_err());
    }

    #[tokio::test]
    async fn test_advanced_composition() {
        // User profile validation: object with required fields, specific types, and constraints
        let user_validator = object()
            .and(has_all_keys(vec!["username".to_string(), "email".to_string(), "age".to_string()]));

        let user_obj = json!({
            "username": "alice123",
            "email": "alice@example.com",
            "age": 25
        });

        assert!(user_validator.validate(&user_obj, None).await.is_ok());

        // Complex string validation: username must be alphanumeric, 3-20 chars, lowercase
        let username_validator = string()
            .and(min_length(3))
            .and(max_length(20))
            .and(alphanumeric(false))
            .and(lowercase());

        assert!(username_validator.validate(&json!("alice123"), None).await.is_ok());
        assert!(username_validator.validate(&json!("Alice123"), None).await.is_err()); // Not lowercase
        assert!(username_validator.validate(&json!("a!"), None).await.is_err()); // Too short + special char

        // Array of valid items
        let numbers_validator = array()
            .and(min_size(1))
            .and(max_size(10));

        let valid_numbers = json!([1, 2, 3, 4, 5]);
        assert!(numbers_validator.validate(&valid_numbers, None).await.is_ok());

        let empty_array = json!([]);
        assert!(numbers_validator.validate(&empty_array, None).await.is_err());
    }

    #[tokio::test]
    async fn test_dimension_validators() {
        // Divisible by
        let divisible_validator = divisible_by(3.0);
        assert!(divisible_validator.validate(&json!(9), None).await.is_ok());
        assert!(divisible_validator.validate(&json!(10), None).await.is_err());

        // Even/Odd
        let even_validator = even();
        assert!(even_validator.validate(&json!(4), None).await.is_ok());
        assert!(even_validator.validate(&json!(5), None).await.is_err());

        let odd_validator = odd();
        assert!(odd_validator.validate(&json!(5), None).await.is_ok());
        assert!(odd_validator.validate(&json!(4), None).await.is_err());
    }

    #[tokio::test]
    async fn test_file_validators() {
        // MIME type
        let image_mime = mime_types(vec!["image/jpeg", "image/png"]);
        assert!(image_mime.validate(&json!("image/jpeg"), None).await.is_ok());
        assert!(image_mime.validate(&json!("text/plain"), None).await.is_err());

        // File extension
        let image_ext = file_extensions(vec!["jpg", "png", "gif"]);
        assert!(image_ext.validate(&json!("photo.jpg"), None).await.is_ok());
        assert!(image_ext.validate(&json!("document.pdf"), None).await.is_err());

        // File size
        let size_validator = file_size_range(100, 5000);
        assert!(size_validator.validate(&json!(2500), None).await.is_ok());
        assert!(size_validator.validate(&json!(50), None).await.is_err());
        assert!(size_validator.validate(&json!(10000), None).await.is_err());
    }

    #[tokio::test]
    async fn test_collection_enhancements() {
        // Distinct values
        let distinct_validator = unique();
        let unique_array = json!([1, 2, 3, 4, 5]);
        assert!(distinct_validator.validate(&unique_array, None).await.is_ok());

        let duplicate_array = json!([1, 2, 3, 2, 5]);
        assert!(distinct_validator.validate(&duplicate_array, None).await.is_err());

        // Simplified element validation - test individual validators
        let positive_validator = positive();
        let positive_number = json!(5);
        assert!(positive_validator.validate(&positive_number, None).await.is_ok());

        let negative_number = json!(-3);
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

        assert!(validator.validate(&json!("user123"), None).await.is_ok());
        assert!(validator.validate(&json!("hi"), None).await.is_err());
        assert_eq!(validator.name(), "username_validator");

        // Test AND logic
        let and_validator = string().and(min_length(5));
        assert!(and_validator.validate(&json!("hello"), None).await.is_ok());
        assert!(and_validator.validate(&json!("hi"), None).await.is_err());
        assert!(and_validator.validate(&json!(123), None).await.is_err());

        // Test OR logic
        let or_validator = string().or(number());
        assert!(or_validator.validate(&json!("hello"), None).await.is_ok());
        assert!(or_validator.validate(&json!(123), None).await.is_ok());
        assert!(or_validator.validate(&json!(true), None).await.is_err());

        // Test NOT logic
        let not_validator = string().not();
        assert!(not_validator.validate(&json!(123), None).await.is_ok());
        assert!(not_validator.validate(&json!("hello"), None).await.is_err());
    }
}