//! Integration tests for nebula-validator
//!
//! These tests verify the public API works correctly

use nebula_validator::combinators::and::and;
use nebula_validator::core::{TypedValidator, ValidationError};
use nebula_validator::validators::string::{max_length, min_length};

#[test]
fn test_min_length_validator() {
    let validator = min_length(5);

    assert!(validator.validate("hello").is_ok());
    assert!(validator.validate("world!").is_ok());
    assert!(validator.validate("hi").is_err());
}

#[test]
fn test_max_length_validator() {
    let validator = max_length(10);

    assert!(validator.validate("hello").is_ok());
    assert!(validator.validate("short").is_ok());
    assert!(validator.validate("this is way too long").is_err());
}

#[test]
fn test_and_combinator() {
    let validator = and(min_length(3), max_length(10));

    assert!(validator.validate("hello").is_ok());
    assert!(validator.validate("hi").is_err()); // too short
    assert!(validator.validate("verylongstring").is_err()); // too long
}

#[test]
fn test_error_messages() {
    let validator = min_length(5);

    match validator.validate("hi") {
        Err(e) => {
            assert_eq!(e.code, "min_length");
            assert!(e.message.contains("5"));
        }
        Ok(_) => panic!("Expected error"),
    }
}

#[test]
fn test_nebula_error_integration() {
    use nebula_validator::NebulaError;

    let validator = min_length(5);
    let result: Result<(), ValidationError> = validator.validate("hi");

    // Convert to NebulaError
    let nebula_err: NebulaError = result.unwrap_err().into();
    let error_string = nebula_err.to_string();

    assert!(error_string.contains("min_length"));
}
