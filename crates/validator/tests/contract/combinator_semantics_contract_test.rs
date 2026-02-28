use nebula_validator::combinators::{unless, when};
use nebula_validator::foundation::{Validate, ValidateExt, ValidationError};
use nebula_validator::validators::{max_length, min_length};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct CountingValidator {
    name: &'static str,
    should_pass: bool,
    calls: Arc<AtomicUsize>,
}

impl Validate<str> for CountingValidator {
    fn validate(&self, _input: &str) -> Result<(), ValidationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.should_pass {
            Ok(())
        } else {
            Err(ValidationError::new(self.name, "forced failure"))
        }
    }
}

#[test]
fn and_short_circuits_on_first_failure() {
    let left_calls = Arc::new(AtomicUsize::new(0));
    let right_calls = Arc::new(AtomicUsize::new(0));
    let left = CountingValidator {
        name: "left_failed",
        should_pass: false,
        calls: left_calls.clone(),
    };
    let right = CountingValidator {
        name: "right_must_not_run",
        should_pass: true,
        calls: right_calls.clone(),
    };

    let and_chain = left.and(right);
    let err = and_chain
        .validate("payload")
        .expect_err("left validator should fail");
    assert_eq!(err.code.as_ref(), "left_failed");
    assert_eq!(left_calls.load(Ordering::SeqCst), 1);
    assert_eq!(right_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn or_short_circuits_on_first_success() {
    let left_calls = Arc::new(AtomicUsize::new(0));
    let right_calls = Arc::new(AtomicUsize::new(0));
    let left = CountingValidator {
        name: "left_ok",
        should_pass: true,
        calls: left_calls.clone(),
    };
    let right = CountingValidator {
        name: "right_must_not_run",
        should_pass: false,
        calls: right_calls.clone(),
    };

    let or_chain = left.or(right);
    assert!(or_chain.validate("payload").is_ok());
    assert_eq!(left_calls.load(Ordering::SeqCst), 1);
    assert_eq!(right_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn not_inverts_validation_result_deterministically() {
    let forbidden = min_length(3).not();
    assert!(forbidden.validate("ok").is_ok());
    assert!(forbidden.validate("long").is_err());
}

#[test]
fn when_and_unless_apply_expected_branches() {
    let only_prefixed = when(min_length(5), |s: &str| s.starts_with("pre_"));
    assert!(only_prefixed.validate("short").is_ok());
    assert!(only_prefixed.validate("pre_x").is_ok());
    assert!(only_prefixed.validate("pre_").is_err());

    let skip_internal = unless(max_length(5), |s: &str| s.starts_with("internal:"));
    assert!(skip_internal.validate("internal:verylong").is_ok());
    assert!(skip_internal.validate("public:verylong").is_err());
}
