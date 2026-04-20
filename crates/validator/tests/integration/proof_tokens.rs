//! Scenario: `Validated<T>` proof tokens.
//!
//! `Validated<T>` carries a compile-time guarantee that a value has passed
//! a specific validator. These tests anchor the zero-cost and
//! trust-boundary contracts and show the idiomatic way to thread a proof
//! through a typed API.

use nebula_validator::{
    foundation::{Validate, ValidationError},
    proof::Validated,
    validators::{email, in_range, min_length},
};

/// Downstream API that only accepts *already-validated* identifiers.
/// Callers cannot fabricate a `Validated<String>` without going through
/// `Validate::validate_into` or `Validated::new`.
fn greet(name: Validated<String>) -> String {
    format!("Hello, {}", name.as_ref())
}

#[test]
fn validated_has_zero_memory_overhead() {
    // The type system guarantee is carried in a zero-sized marker, so the
    // runtime footprint matches the wrapped type exactly.
    assert_eq!(size_of::<Validated<String>>(), size_of::<String>());
    assert_eq!(size_of::<Validated<u64>>(), size_of::<u64>());
}

#[test]
fn validate_into_produces_validated_on_success() {
    let validator = min_length(3);
    let proof: Validated<String> = validator
        .validate_into("alice".to_string())
        .expect("min length satisfied");
    assert_eq!(proof.as_ref(), "alice");
}

#[test]
fn validate_into_propagates_error_on_failure() {
    let validator = min_length(5);
    let result: Result<Validated<String>, _> = validator.validate_into("hi".to_string());
    assert!(result.is_err());
}

#[test]
fn validated_new_runs_the_validator() {
    let good = Validated::new("alice".to_string(), &min_length(3));
    assert!(good.is_ok());

    let bad = Validated::new("ab".to_string(), &min_length(5));
    assert!(bad.is_err());
}

#[test]
fn validated_threads_through_typed_api() {
    // Downstream `greet` requires a `Validated<String>` — the type system
    // prevents passing an unvalidated string, so the check always runs.
    let proof = min_length(3).validate_into("world".to_string()).unwrap();
    let greeting = greet(proof);
    assert_eq!(greeting, "Hello, world");
}

#[test]
fn validated_supports_into_inner_for_ownership_recovery() {
    let proof: Validated<String> = min_length(3).validate_into("alice".to_string()).unwrap();
    let owned: String = proof.into_inner();
    assert_eq!(owned, "alice");
}

#[test]
fn validated_equality_reflects_wrapped_value() {
    let a: Validated<String> = min_length(1).validate_into("x".to_string()).unwrap();
    let b: Validated<String> = min_length(1).validate_into("x".to_string()).unwrap();
    let c: Validated<String> = min_length(1).validate_into("y".to_string()).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn validated_carries_hash_for_use_in_sets() {
    use std::collections::HashSet;
    let proof_a: Validated<String> = min_length(1).validate_into("x".to_string()).unwrap();
    let proof_b: Validated<String> = min_length(1).validate_into("x".to_string()).unwrap();
    let mut set: HashSet<Validated<String>> = HashSet::new();
    set.insert(proof_a);
    // Same underlying value; should deduplicate.
    assert!(!set.insert(proof_b));
}

#[test]
fn multiple_validators_chain_via_validated() {
    // First ensure the string is non-empty; then upgrade to an email-validated
    // proof. Each step returns a Validated guaranteed by the next-level validator.
    let initial = min_length(1)
        .validate_into("user@example.com".to_string())
        .expect("non-empty");
    let email_proof: Result<Validated<String>, _> = email().validate_into(initial.into_inner());
    assert!(email_proof.is_ok());
}

#[test]
fn numeric_proof_from_range_validator() {
    let valid: Result<Validated<i32>, _> = in_range(0i32, 100i32).validate_into(42);
    assert!(valid.is_ok());

    let out: Result<Validated<i32>, ValidationError> = in_range(0i32, 100i32)
        .validate_into(101)
        .map_err(|e| match e {
            nebula_validator::ValidatorError::ValidationFailed(inner) => inner,
            other => panic!("unexpected error variant: {other:?}"),
        });
    assert!(out.is_err());
}
