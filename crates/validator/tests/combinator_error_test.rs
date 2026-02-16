use nebula_validator::combinators::error::CombinatorError;
use nebula_validator::foundation::ValidationError;

#[test]
fn test_or_all_failed() {
    let err1 = ValidationError::new("err1", "First error");
    let err2 = ValidationError::new("err2", "Second error");
    let error = CombinatorError::or_all_failed(err1, err2);

    assert!(matches!(error, CombinatorError::OrAllFailed { .. }));
    let display = format!("{}", error);
    assert!(display.contains("All validators failed"));
}

#[test]
fn test_field_failed() {
    let err = ValidationError::new("invalid", "Invalid value");
    let error = CombinatorError::field_failed("email", err);

    assert!(error.is_field_error());
    assert_eq!(error.field_name(), Some("email"));
    let display = format!("{}", error);
    assert!(display.contains("field 'email'"));
}

#[test]
fn test_required_missing() {
    let error: CombinatorError<ValidationError> = CombinatorError::required_missing();
    let display = format!("{}", error);
    assert!(display.contains("required"));
}

#[test]
fn test_not_passed() {
    let error: CombinatorError<ValidationError> = CombinatorError::not_passed();
    let display = format!("{}", error);
    assert!(display.contains("must NOT pass"));
}

#[test]
fn test_multiple_failed() {
    let errors = vec![
        ValidationError::new("err1", "Error 1"),
        ValidationError::new("err2", "Error 2"),
    ];
    let error = CombinatorError::multiple_failed(errors);

    assert!(error.is_multiple());
    let display = format!("{}", error);
    assert!(display.contains("2 errors"));
}

#[test]
fn test_custom_error() {
    let error: CombinatorError<ValidationError> =
        CombinatorError::custom("custom_code", "Custom message");
    let display = format!("{}", error);
    assert!(display.contains("custom_code"));
    assert!(display.contains("Custom message"));
}

#[test]
fn test_conversion_to_validation_error() {
    let error: CombinatorError<ValidationError> = CombinatorError::required_missing();
    let ve: ValidationError = error.into();
    assert_eq!(ve.code, "required");
}

#[test]
fn test_conversion_from_validation_error() {
    let ve = ValidationError::new("test", "Test error");
    let error: CombinatorError<ValidationError> = ve.into();
    assert!(matches!(error, CombinatorError::ValidationFailed(_)));
}

#[test]
fn test_and_failed() {
    let err = ValidationError::new("test", "Test failed");
    let error = CombinatorError::and_failed(err);
    let display = format!("{}", error);
    assert!(display.contains("AND combinator failed"));
}

#[test]
fn test_field_failed_unnamed() {
    let err = ValidationError::new("test", "Test error");
    let error = CombinatorError::field_failed_unnamed(err);
    assert!(error.is_field_error());
    assert_eq!(error.field_name(), None);
}

#[test]
fn test_error_source() {
    let err = ValidationError::new("inner", "Inner error");
    let error = CombinatorError::validation_failed(err);

    use std::error::Error;
    assert!(error.source().is_some());
}

#[test]
fn test_multiple_errors_with_conversion() {
    let errors = vec![
        ValidationError::new("err1", "Error 1"),
        ValidationError::new("err2", "Error 2"),
        ValidationError::new("err3", "Error 3"),
    ];
    let error = CombinatorError::multiple_failed(errors);

    let ve: ValidationError = error.into();
    assert_eq!(ve.code, "multiple_failures");
    assert_eq!(ve.nested.len(), 3);
}

#[test]
fn test_field_error_with_conversion() {
    let err = ValidationError::new("email_invalid", "Invalid email format");
    let error = CombinatorError::field_failed("user.email", err);

    let ve: ValidationError = error.into();
    assert_eq!(ve.code, "field_validation_failed");
    assert_eq!(ve.field.as_deref(), Some("user.email"));
}
