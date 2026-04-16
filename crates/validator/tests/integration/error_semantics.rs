//! Scenario: end-to-end error semantics — field pointers, nesting depth,
//! parameter redaction, and severity — across the full validation stack.

use nebula_validator::{
    Validator,
    foundation::{ErrorSeverity, Validate, ValidationError, ValidationErrors},
};

use super::common::{assert_has_code, expect_errors, find_by_field};

#[derive(Validator)]
struct Credentials {
    #[validate(min_length = 1)]
    username: String,

    #[validate(min_length = 8)]
    password: String,
}

#[test]
fn field_pointers_are_rfc6901_normalized() {
    let creds = Credentials {
        username: "".into(),
        password: "".into(),
    };
    let errors = expect_errors(creds.validate_fields());
    // Field pointers normalise to `/field`, not `field` or `.field`.
    assert!(find_by_field(&errors, "/username").is_some());
    assert!(find_by_field(&errors, "/password").is_some());
}

#[test]
fn sensitive_parameter_names_are_redacted() {
    // Construct a ValidationError that carries a sensitive parameter; the
    // `with_param` helper redacts known-sensitive keys (password, token, …).
    let err = ValidationError::new("auth_failed", "authentication failed")
        .with_param("password", "super-secret")
        .with_param("api_key", "abc123")
        .with_param("username", "alice");

    assert_eq!(err.param("password"), Some("[REDACTED]"));
    assert_eq!(err.param("api_key"), Some("[REDACTED]"));
    assert_eq!(err.param("username"), Some("alice"));
}

#[test]
fn severity_defaults_to_error_and_survives_nesting() {
    let root = ValidationError::new("root", "root")
        .with_severity(ErrorSeverity::Warning)
        .with_nested(vec![
            ValidationError::new("child_a", "a"),
            ValidationError::new("child_b", "b").with_severity(ErrorSeverity::Info),
        ]);

    // Parent severity is preserved on the root…
    assert_eq!(root.severity(), ErrorSeverity::Warning);
    // …child severities are independent.
    assert_eq!(root.nested()[0].severity(), ErrorSeverity::Error);
    assert_eq!(root.nested()[1].severity(), ErrorSeverity::Info);
}

#[test]
fn flatten_walks_entire_tree_depth_first() {
    let err = ValidationError::new("l0", "l0").with_nested(vec![
        ValidationError::new("l1a", "l1a").with_nested(vec![ValidationError::new("l2", "l2")]),
        ValidationError::new("l1b", "l1b"),
    ]);
    // 1 root + 2 first-level + 1 grandchild = 4 total.
    assert_eq!(err.total_error_count(), 4);
    let flat = err.flatten();
    let codes: Vec<&str> = flat.iter().map(|e| e.code.as_ref()).collect();
    assert_eq!(codes, &["l0", "l1a", "l2", "l1b"]);
}

#[test]
fn validate_fields_returns_collection_validate_returns_single() {
    let creds = Credentials {
        username: "".into(),
        password: "".into(),
    };

    // Raw collection keeps each field failure independent.
    let multi: ValidationErrors = creds.validate_fields().unwrap_err();
    assert_eq!(multi.errors().len(), 2);

    // Single-error surface collapses them with the nested tree preserved.
    let single: ValidationError = creds.validate(&creds).unwrap_err();
    assert_eq!(single.nested().len(), 2);
    assert_has_code(
        &ValidationErrors::from_iter(single.nested().iter().cloned()),
        "min_length",
    );
}
