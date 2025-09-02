//! Tests for string validators

use nebula_validator::{
    Validatable, ValidationResult, ValidationError, ErrorCode,
    validators::string::{StringLength, Pattern, Email, Url, Uuid},
};
use serde_json::Value;
use crate::common::*;

#[tokio::test]
async fn test_string_length_validator() {
    // Test minimum length
    let validator = StringLength::new(Some(3), None);
    
    let short_string = serde_json::json!("ab");
    let result = validator.validate(&short_string).await;
    assert_validation_failure(&result);
    
    let valid_string = serde_json::json!("abc");
    let result = validator.validate(&valid_string).await;
    assert_validation_success(&result);
    
    let long_string = serde_json::json!("abcdef");
    let result = validator.validate(&long_string).await;
    assert_validation_success(&result);
    
    // Test maximum length
    let validator = StringLength::new(None, Some(5));
    
    let short_string = serde_json::json!("ab");
    let result = validator.validate(&short_string).await;
    assert_validation_success(&result);
    
    let valid_string = serde_json::json!("abcde");
    let result = validator.validate(&valid_string).await;
    assert_validation_success(&result);
    
    let long_string = serde_json::json!("abcdef");
    let result = validator.validate(&long_string).await;
    assert_validation_failure(&result);
    
    // Test range
    let validator = StringLength::new(Some(3), Some(5));
    
    let short_string = serde_json::json!("ab");
    let result = validator.validate(&short_string).await;
    assert_validation_failure(&result);
    
    let valid_string = serde_json::json!("abc");
    let result = validator.validate(&valid_string).await;
    assert_validation_success(&result);
    
    let valid_string2 = serde_json::json!("abcde");
    let result = validator.validate(&valid_string2).await;
    assert_validation_success(&result);
    
    let long_string = serde_json::json!("abcdef");
    let result = validator.validate(&long_string).await;
    assert_validation_failure(&result);
    
    // Test non-string values
    let validator = StringLength::new(Some(3), Some(5));
    
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure(&result);
    
    let array_value = serde_json::json!([1, 2, 3]);
    let result = validator.validate(&array_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_pattern_validator() {
    // Test email-like pattern
    let validator = Pattern::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$");
    
    let valid_email = serde_json::json!("user@example.com");
    let result = validator.validate(&valid_email).await;
    assert_validation_success(&result);
    
    let invalid_email = serde_json::json!("invalid-email");
    let result = validator.validate(&invalid_email).await;
    assert_validation_failure(&result);
    
    // Test alphanumeric pattern
    let validator = Pattern::new(r"^[a-zA-Z0-9]+$");
    
    let valid_alphanumeric = serde_json::json!("abc123");
    let result = validator.validate(&valid_alphanumeric).await;
    assert_validation_success(&result);
    
    let invalid_alphanumeric = serde_json::json!("abc-123");
    let result = validator.validate(&invalid_alphanumeric).await;
    assert_validation_failure(&result);
    
    // Test non-string values
    let validator = Pattern::new(r"^[a-zA-Z]+$");
    
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_email_validator() {
    let validator = Email::new();
    
    // Valid emails
    let valid_emails = vec![
        serde_json::json!("user@example.com"),
        serde_json::json!("user.name@example.com"),
        serde_json::json!("user+tag@example.com"),
        serde_json::json!("user@subdomain.example.com"),
        serde_json::json!("user@example.co.uk"),
    ];
    
    for email in valid_emails {
        let result = validator.validate(&email).await;
        assert_validation_success(&result);
    }
    
    // Invalid emails
    let invalid_emails = vec![
        serde_json::json!("invalid-email"),
        serde_json::json!("user@"),
        serde_json::json!("@example.com"),
        serde_json::json!("user@.com"),
        serde_json::json!("user..name@example.com"),
        serde_json::json!("user@example..com"),
    ];
    
    for email in invalid_emails {
        let result = validator.validate(&email).await;
        assert_validation_failure(&result);
    }
    
    // Test non-string values
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_url_validator() {
    let validator = Url::new();
    
    // Valid URLs
    let valid_urls = vec![
        serde_json::json!("https://example.com"),
        serde_json::json!("http://example.com"),
        serde_json::json!("https://example.com/path"),
        serde_json::json!("https://example.com/path?param=value"),
        serde_json::json!("https://subdomain.example.com"),
        serde_json::json!("ftp://example.com"),
    ];
    
    for url in valid_urls {
        let result = validator.validate(&url).await;
        assert_validation_success(&result);
    }
    
    // Invalid URLs
    let invalid_urls = vec![
        serde_json::json!("not-a-url"),
        serde_json::json!("example.com"),
        serde_json::json!("http://"),
        serde_json::json!("https://"),
    ];
    
    for url in invalid_urls {
        let result = validator.validate(&url).await;
        assert_validation_failure(&result);
    }
    
    // Test non-string values
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_uuid_validator() {
    let validator = Uuid::new();
    
    // Valid UUIDs
    let valid_uuids = vec![
        serde_json::json!("550e8400-e29b-41d4-a716-446655440000"),
        serde_json::json!("550e8400-e29b-41d4-a716-446655440001"),
        serde_json::json!("00000000-0000-0000-0000-000000000000"),
        serde_json::json!("ffffffff-ffff-ffff-ffff-ffffffffffff"),
    ];
    
    for uuid in valid_uuids {
        let result = validator.validate(&uuid).await;
        assert_validation_success(&result);
    }
    
    // Invalid UUIDs
    let invalid_uuids = vec![
        serde_json::json!("not-a-uuid"),
        serde_json::json!("550e8400-e29b-41d4-a716-44665544000"), // Too short
        serde_json::json!("550e8400-e29b-41d4-a716-4466554400000"), // Too long
        serde_json::json!("550e8400-e29b-41d4-a716-44665544000g"), // Invalid character
    ];
    
    for uuid in invalid_uuids {
        let result = validator.validate(&uuid).await;
        assert_validation_failure(&result);
    }
    
    // Test non-string values
    let number_value = serde_json::json!(42);
    let result = validator.validate(&number_value).await;
    assert_validation_failure(&result);
}

#[tokio::test]
async fn test_string_validator_metadata() {
    let length_validator = StringLength::new(Some(3), Some(5));
    let metadata = length_validator.metadata();
    
    assert_eq!(metadata.id, "string_length");
    assert_eq!(metadata.name, "String Length");
    assert_eq!(metadata.category, ValidatorCategory::String);
    
    let pattern_validator = Pattern::new(r"^[a-z]+$");
    let metadata = pattern_validator.metadata();
    
    assert_eq!(metadata.id, "pattern");
    assert_eq!(metadata.name, "Pattern");
    assert_eq!(metadata.category, ValidatorCategory::String);
    
    let email_validator = Email::new();
    let metadata = email_validator.metadata();
    
    assert_eq!(metadata.id, "email");
    assert_eq!(metadata.name, "Email");
    assert_eq!(metadata.category, ValidatorCategory::Format);
    
    let url_validator = Url::new();
    let metadata = url_validator.metadata();
    
    assert_eq!(metadata.id, "url");
    assert_eq!(metadata.name, "URL");
    assert_eq!(metadata.category, ValidatorCategory::Format);
    
    let uuid_validator = Uuid::new();
    let metadata = uuid_validator.metadata();
    
    assert_eq!(metadata.id, "uuid");
    assert_eq!(metadata.name, "UUID");
    assert_eq!(metadata.category, ValidatorCategory::Format);
}

#[tokio::test]
async fn test_string_validator_complexity() {
    let length_validator = StringLength::new(Some(3), Some(5));
    assert_eq!(length_validator.complexity(), ValidationComplexity::Simple);
    
    let pattern_validator = Pattern::new(r"^[a-z]+$");
    assert_eq!(pattern_validator.complexity(), ValidationComplexity::Simple);
    
    let email_validator = Email::new();
    assert_eq!(email_validator.complexity(), ValidationComplexity::Simple);
    
    let url_validator = Url::new();
    assert_eq!(url_validator.complexity(), ValidationComplexity::Simple);
    
    let uuid_validator = Uuid::new();
    assert_eq!(uuid_validator.complexity(), ValidationComplexity::Simple);
}

#[tokio::test]
async fn test_string_validator_accepts() {
    let length_validator = StringLength::new(Some(3), Some(5));
    let pattern_validator = Pattern::new(r"^[a-z]+$");
    let email_validator = Email::new();
    let url_validator = Url::new();
    let uuid_validator = Uuid::new();
    
    // String validators should only accept string values
    let string_value = serde_json::json!("test");
    let number_value = serde_json::json!(42);
    
    assert!(length_validator.accepts(&string_value));
    assert!(!length_validator.accepts(&number_value));
    
    assert!(pattern_validator.accepts(&string_value));
    assert!(!pattern_validator.accepts(&number_value));
    
    assert!(email_validator.accepts(&string_value));
    assert!(!email_validator.accepts(&number_value));
    
    assert!(url_validator.accepts(&string_value));
    assert!(!url_validator.accepts(&number_value));
    
    assert!(uuid_validator.accepts(&string_value));
    assert!(!uuid_validator.accepts(&number_value));
}

#[tokio::test]
async fn test_string_validator_estimate_time() {
    let length_validator = StringLength::new(Some(3), Some(5));
    let pattern_validator = Pattern::new(r"^[a-z]+$");
    let email_validator = Email::new();
    let url_validator = Url::new();
    let uuid_validator = Uuid::new();
    
    let short_string = serde_json::json!("abc");
    let long_string = serde_json::json!("very long string for testing");
    
    // Test short strings
    let length_time_short = length_validator.estimate_time_ms(&short_string);
    let pattern_time_short = pattern_validator.estimate_time_ms(&short_string);
    let email_time_short = email_validator.estimate_time_ms(&short_string);
    let url_time_short = url_validator.estimate_time_ms(&short_string);
    let uuid_time_short = uuid_validator.estimate_time_ms(&short_string);
    
    assert!(length_time_short > 0);
    assert!(pattern_time_short > 0);
    assert!(email_time_short > 0);
    assert!(url_time_short > 0);
    assert!(uuid_time_short > 0);
    
    // Test long strings
    let length_time_long = length_validator.estimate_time_ms(&long_string);
    let pattern_time_long = pattern_validator.estimate_time_ms(&long_string);
    let email_time_long = email_validator.estimate_time_ms(&long_string);
    let url_time_long = url_validator.estimate_time_ms(&long_string);
    let uuid_time_long = uuid_validator.estimate_time_ms(&long_string);
    
    assert!(length_time_long > 0);
    assert!(pattern_time_long > 0);
    assert!(email_time_long > 0);
    assert!(url_time_long > 0);
    assert!(uuid_time_long > 0);
    
    // Long strings should take more time than short strings
    assert!(length_time_long >= length_time_short);
    assert!(pattern_time_long >= pattern_time_short);
    assert!(email_time_long >= email_time_short);
    assert!(url_time_long >= url_time_short);
    assert!(uuid_time_long >= uuid_time_short);
}
