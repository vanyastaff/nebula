//! Tests for Refined types

use nebula_validator::core::{TypedValidator, ValidationError, Refined};
use nebula_validator::validators::string::min_length;

#[test]
fn test_refined_valid() {
    let validator = min_length(5);
    let refined = Refined::new("hello world".to_string(), &validator);

    assert!(refined.is_ok());
    let refined = refined.unwrap();
    assert_eq!(refined.as_ref(), "hello world");
}

#[test]
fn test_refined_invalid() {
    let validator = min_length(5);
    let refined = Refined::new("hi".to_string(), &validator);

    assert!(refined.is_err());
}

#[test]
fn test_refined_with_str() {
    let validator = min_length(3);
    let refined = Refined::new("hello".to_string(), &validator);

    assert!(refined.is_ok());
}

#[test]
fn test_refined_into_inner() {
    let validator = min_length(5);
    let refined = Refined::new("hello".to_string(), &validator).unwrap();
    let inner: String = refined.into_inner();

    assert_eq!(inner, "hello");
}
