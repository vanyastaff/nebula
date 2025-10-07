//! Integration tests for #[derive(Validator)]

use nebula_derive::Validator;
use nebula_validator::core::ValidationErrors;

// Simple test struct
#[derive(Validator)]
struct LoginForm {
    #[validate(min_length = 3, max_length = 20)]
    username: String,

    #[validate(min_length = 8)]
    password: String,
}

#[test]
fn test_derive_validator_basic() {
    let form = LoginForm {
        username: "alice".to_string(),
        password: "secret123".to_string(),
    };

    assert!(form.validate().is_ok());
}

#[test]
fn test_derive_validator_username_too_short() {
    let form = LoginForm {
        username: "ab".to_string(), // Too short!
        password: "secret123".to_string(),
    };

    assert!(form.validate().is_err());
}

#[test]
fn test_derive_validator_username_too_long() {
    let form = LoginForm {
        username: "a".repeat(30), // Too long!
        password: "secret123".to_string(),
    };

    assert!(form.validate().is_err());
}

#[test]
fn test_derive_validator_password_too_short() {
    let form = LoginForm {
        username: "alice".to_string(),
        password: "short".to_string(), // Too short!
    };

    assert!(form.validate().is_err());
}

// Test with numeric validators
#[derive(Validator)]
struct UserProfile {
    #[validate(min_length = 1)]
    name: String,

    #[validate(range(min = 18, max = 100))]
    age: u8,
}

#[test]
fn test_numeric_validator() {
    let profile = UserProfile {
        name: "Bob".to_string(),
        age: 25,
    };

    assert!(profile.validate().is_ok());
}

#[test]
fn test_numeric_validator_too_young() {
    let profile = UserProfile {
        name: "Bob".to_string(),
        age: 15, // Too young!
    };

    assert!(profile.validate().is_err());
}

#[test]
fn test_numeric_validator_too_old() {
    let profile = UserProfile {
        name: "Bob".to_string(),
        age: 150, // Too old!
    };

    assert!(profile.validate().is_err());
}

// Test with skip attribute
#[derive(Validator)]
struct PartialValidation {
    #[validate(min_length = 3)]
    validated_field: String,

    #[validate(skip)]
    skipped_field: String,
}

#[test]
fn test_skip_attribute() {
    let data = PartialValidation {
        validated_field: "abc".to_string(),
        skipped_field: "x".to_string(), // Would fail if not skipped
    };

    assert!(data.validate().is_ok());
}
