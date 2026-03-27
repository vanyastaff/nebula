//! Integration tests for serde serialization.
#![cfg(feature = "serde")]

use nebula_error::{ErrorCategory, ErrorCode, ErrorSeverity};

#[test]
fn severity_roundtrip() {
    let json = serde_json::to_string(&ErrorSeverity::Warning).unwrap();
    assert_eq!(json, "\"warning\"");
    let back: ErrorSeverity = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorSeverity::Warning);
}

#[test]
fn category_roundtrip() {
    let json = serde_json::to_string(&ErrorCategory::RateLimit).unwrap();
    assert_eq!(json, "\"rate_limit\"");
    let back: ErrorCategory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ErrorCategory::RateLimit);
}

#[test]
fn error_code_roundtrip() {
    let code = ErrorCode::new("MY_CODE");
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"MY_CODE\"");
    let back: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.as_str(), "MY_CODE");
}

#[test]
fn all_categories_roundtrip() {
    let categories = [
        ErrorCategory::NotFound,
        ErrorCategory::Validation,
        ErrorCategory::Authentication,
        ErrorCategory::Authorization,
        ErrorCategory::Conflict,
        ErrorCategory::RateLimit,
        ErrorCategory::Timeout,
        ErrorCategory::Exhausted,
        ErrorCategory::Cancelled,
        ErrorCategory::Internal,
        ErrorCategory::External,
        ErrorCategory::Unsupported,
    ];
    for cat in categories {
        let json = serde_json::to_string(&cat).unwrap();
        let back: ErrorCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cat, "roundtrip failed for {cat:?}");
    }
}

#[test]
fn all_severities_roundtrip() {
    for sev in [
        ErrorSeverity::Error,
        ErrorSeverity::Warning,
        ErrorSeverity::Info,
    ] {
        let json = serde_json::to_string(&sev).unwrap();
        let back: ErrorSeverity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sev);
    }
}
