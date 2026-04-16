//! Shared helpers for integration scenarios.
//!
//! Kept intentionally small: scenarios should remain self-explanatory, so
//! only truly generic assertions live here.

#![allow(dead_code)]

use nebula_validator::foundation::{ValidationError, ValidationErrors};

/// Asserts the result is `Err` and returns the collected errors.
pub fn expect_errors<T: std::fmt::Debug>(result: Result<T, ValidationErrors>) -> ValidationErrors {
    match result {
        Ok(ok) => panic!("expected validation to fail, got Ok({ok:?})"),
        Err(errors) => errors,
    }
}

/// Asserts that the given error code is present somewhere in the error
/// list. Useful when multiple errors accumulate and the test only cares
/// about a specific one.
pub fn assert_has_code(errors: &ValidationErrors, code: &str) {
    let found = errors.errors().iter().any(|e| e.code.as_ref() == code);
    assert!(
        found,
        "expected code `{code}` in errors, got: [{}]",
        error_code_list(errors)
    );
}

/// Asserts the error set covers exactly the given codes (order-independent,
/// duplicates allowed in either side).
pub fn assert_codes_exactly(errors: &ValidationErrors, expected: &[&str]) {
    let mut actual: Vec<&str> = errors.errors().iter().map(|e| e.code.as_ref()).collect();
    let mut expected: Vec<&str> = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(
        actual,
        expected,
        "error-code sets differ. got: [{}]",
        error_code_list(errors)
    );
}

/// Returns a comma-separated list of error codes — handy for panic messages.
pub fn error_code_list(errors: &ValidationErrors) -> String {
    errors
        .errors()
        .iter()
        .map(|e| e.code.as_ref())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the first error whose field pointer matches `field`, if any.
pub fn find_by_field<'a>(errors: &'a ValidationErrors, field: &str) -> Option<&'a ValidationError> {
    errors
        .errors()
        .iter()
        .find(|e| e.field.as_deref() == Some(field))
}
