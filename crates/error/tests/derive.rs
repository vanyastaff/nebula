use std::time::Duration;

use nebula_error::{Classify, ErrorCategory, ErrorSeverity};

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum SimpleError {
    #[classify(category = "timeout", code = "SIMPLE_TIMEOUT")]
    #[error("timed out")]
    Timeout,

    #[classify(category = "validation", code = "SIMPLE_INVALID")]
    #[error("invalid")]
    Invalid,
}

#[test]
fn simple_category() {
    assert_eq!(SimpleError::Timeout.category(), ErrorCategory::Timeout);
    assert_eq!(SimpleError::Invalid.category(), ErrorCategory::Validation);
}

#[test]
fn simple_error_code() {
    assert_eq!(SimpleError::Timeout.code().as_str(), "SIMPLE_TIMEOUT");
    assert_eq!(SimpleError::Invalid.code().as_str(), "SIMPLE_INVALID");
}

#[test]
fn simple_default_severity() {
    assert_eq!(SimpleError::Timeout.severity(), ErrorSeverity::Error);
}

#[test]
fn simple_default_retryable() {
    assert!(SimpleError::Timeout.is_retryable());
    assert!(!SimpleError::Invalid.is_retryable());
}

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum FullError {
    #[classify(category = "timeout", code = "FULL_TIMEOUT")]
    #[error("timeout")]
    Timeout,

    #[classify(category = "validation", code = "FULL_WARN", severity = "warning")]
    #[error("warning")]
    SoftWarning,

    #[classify(category = "rate_limit", code = "FULL_RATE", retry_after_secs = 60)]
    #[error("rate limited")]
    RateLimited,

    #[classify(category = "external", code = "FULL_EXT", retryable = false)]
    #[error("external")]
    ExternalNonRetryable,
}

#[test]
fn full_severity_override() {
    assert_eq!(FullError::SoftWarning.severity(), ErrorSeverity::Warning);
    assert_eq!(FullError::Timeout.severity(), ErrorSeverity::Error);
}

#[test]
fn full_retryable_override() {
    assert!(!FullError::ExternalNonRetryable.is_retryable());
    assert!(FullError::Timeout.is_retryable());
}

#[test]
fn full_retry_hint() {
    let hint = FullError::RateLimited.retry_hint().unwrap();
    assert_eq!(hint.after, Some(Duration::from_secs(60)));
    assert!(FullError::Timeout.retry_hint().is_none());
}

#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum WithFields {
    #[classify(category = "external", code = "API_ERR")]
    #[error("API error {status}: {body}")]
    ApiError { status: u16, body: String },

    #[classify(category = "timeout", code = "CONN_TIMEOUT")]
    #[error("connection timeout after {0:?}")]
    ConnTimeout(Duration),
}

#[test]
fn variants_with_fields() {
    let err = WithFields::ApiError {
        status: 429,
        body: "too many".into(),
    };
    assert_eq!(err.category(), ErrorCategory::External);
    assert_eq!(err.code().as_str(), "API_ERR");

    let err = WithFields::ConnTimeout(Duration::from_secs(5));
    assert_eq!(err.category(), ErrorCategory::Timeout);
    assert!(err.is_retryable());
}

/// Verify all 12 categories map correctly.
#[derive(Debug, thiserror::Error, nebula_error_macros::Classify)]
enum AllCategories {
    #[classify(category = "not_found", code = "C1")]
    #[error("e")]
    NotFound,
    #[classify(category = "validation", code = "C2")]
    #[error("e")]
    Validation,
    #[classify(category = "authentication", code = "C3")]
    #[error("e")]
    Authentication,
    #[classify(category = "authorization", code = "C4")]
    #[error("e")]
    Authorization,
    #[classify(category = "conflict", code = "C5")]
    #[error("e")]
    Conflict,
    #[classify(category = "rate_limit", code = "C6")]
    #[error("e")]
    RateLimit,
    #[classify(category = "timeout", code = "C7")]
    #[error("e")]
    Timeout,
    #[classify(category = "exhausted", code = "C8")]
    #[error("e")]
    Exhausted,
    #[classify(category = "cancelled", code = "C9")]
    #[error("e")]
    Cancelled,
    #[classify(category = "internal", code = "C10")]
    #[error("e")]
    Internal,
    #[classify(category = "external", code = "C11")]
    #[error("e")]
    External,
    #[classify(category = "unsupported", code = "C12")]
    #[error("e")]
    Unsupported,
}

#[test]
fn all_categories_map_correctly() {
    assert_eq!(AllCategories::NotFound.category(), ErrorCategory::NotFound);
    assert_eq!(
        AllCategories::Validation.category(),
        ErrorCategory::Validation
    );
    assert_eq!(
        AllCategories::Authentication.category(),
        ErrorCategory::Authentication
    );
    assert_eq!(
        AllCategories::Authorization.category(),
        ErrorCategory::Authorization
    );
    assert_eq!(AllCategories::Conflict.category(), ErrorCategory::Conflict);
    assert_eq!(
        AllCategories::RateLimit.category(),
        ErrorCategory::RateLimit
    );
    assert_eq!(AllCategories::Timeout.category(), ErrorCategory::Timeout);
    assert_eq!(
        AllCategories::Exhausted.category(),
        ErrorCategory::Exhausted
    );
    assert_eq!(
        AllCategories::Cancelled.category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(AllCategories::Internal.category(), ErrorCategory::Internal);
    assert_eq!(AllCategories::External.category(), ErrorCategory::External);
    assert_eq!(
        AllCategories::Unsupported.category(),
        ErrorCategory::Unsupported
    );
}
