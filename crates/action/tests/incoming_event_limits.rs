//! Integration tests for `IncomingEvent` body-size and header-count limits
//! introduced in M4 + M5.
//!
//! Cover the DoS-hardening construction path:
//! - oversized body → `DataLimitExceeded`
//! - header count above cap → `Validation`
//! - custom limits via `try_new_with_limits`
//! - case-insensitive O(1) header lookup after lowercase normalization
//! - duplicate key collapse (last write wins)

use nebula_action::{ActionError, DEFAULT_MAX_BODY_BYTES, IncomingEvent, MAX_HEADER_COUNT};

#[test]
fn try_new_accepts_empty_body() {
    assert!(IncomingEvent::try_new(&[], &[]).is_ok());
}

#[test]
fn try_new_accepts_exact_limit_body() {
    let body = vec![0u8; DEFAULT_MAX_BODY_BYTES];
    assert!(IncomingEvent::try_new(&body, &[]).is_ok());
}

#[test]
fn try_new_rejects_oversized_body() {
    let body = vec![0u8; DEFAULT_MAX_BODY_BYTES + 1];
    let err = IncomingEvent::try_new(&body, &[]).unwrap_err();
    match err {
        ActionError::DataLimitExceeded {
            limit_bytes,
            actual_bytes,
        } => {
            assert_eq!(limit_bytes, DEFAULT_MAX_BODY_BYTES as u64);
            assert_eq!(actual_bytes, (DEFAULT_MAX_BODY_BYTES + 1) as u64);
        }
        other => panic!("expected DataLimitExceeded, got {other:?}"),
    }
}

#[test]
fn try_new_with_limits_custom_cap_accepts_under() {
    let body = vec![0u8; 2048];
    assert!(IncomingEvent::try_new_with_limits(&body, &[], 4096, 16).is_ok());
}

#[test]
fn try_new_with_limits_custom_cap_rejects_over() {
    let body = vec![0u8; 2048];
    let err = IncomingEvent::try_new_with_limits(&body, &[], 1024, 16).unwrap_err();
    assert!(matches!(err, ActionError::DataLimitExceeded { .. }));
}

#[test]
fn try_new_accepts_max_header_count() {
    let headers_owned: Vec<(String, String)> = (0..MAX_HEADER_COUNT)
        .map(|i| (format!("x-h{i}"), "v".to_string()))
        .collect();
    let headers: Vec<(&str, &str)> = headers_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert!(IncomingEvent::try_new(b"", &headers).is_ok());
}

#[test]
fn try_new_rejects_too_many_headers() {
    let headers_owned: Vec<(String, String)> = (0..(MAX_HEADER_COUNT + 1))
        .map(|i| (format!("x-h{i}"), "v".to_string()))
        .collect();
    let headers: Vec<(&str, &str)> = headers_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let err = IncomingEvent::try_new(b"", &headers).unwrap_err();
    match err {
        ActionError::Validation {
            field,
            reason,
            detail,
        } => {
            assert_eq!(field, "headers");
            assert_eq!(reason, nebula_action::ValidationReason::OutOfRange);
            let detail = detail.expect("header count error carries detail");
            assert!(detail.contains("too many headers"), "detail was: {detail}");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn try_new_with_limits_custom_header_cap() {
    let err =
        IncomingEvent::try_new_with_limits(b"", &[("a", "1"), ("b", "2"), ("c", "3")], 1024, 2)
            .unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[test]
fn header_lookup_is_case_insensitive() {
    let event = IncomingEvent::try_new(b"", &[("x-custom", "v1"), ("X-Other", "v2")]).unwrap();
    // All-lowercase query — fast path, no allocation.
    assert_eq!(event.header("x-custom"), Some("v1"));
    // Mixed-case query — slow path, one fold allocation.
    assert_eq!(event.header("X-Custom"), Some("v1"));
    assert_eq!(event.header("X-OTHER"), Some("v2"));
    // Missing key.
    assert_eq!(event.header("missing"), None);
}

#[test]
fn header_duplicate_keys_collapse_last_wins() {
    // Both keys normalize to "x-sig". HashMap overwrites on insert,
    // so the final value is whatever was passed last. Document this
    // behavior so consumers are not surprised.
    let event = IncomingEvent::try_new(b"", &[("X-Sig", "first"), ("x-sig", "last")]).unwrap();
    assert_eq!(event.header("X-Sig"), Some("last"));
}

#[test]
fn header_count_cap_beats_body_cap_order() {
    // Body is fine but too many headers — must fail with Validation,
    // not DataLimitExceeded. Order of checks is load-bearing for the
    // test to distinguish which guard fired.
    let headers_owned: Vec<(String, String)> = (0..(MAX_HEADER_COUNT + 1))
        .map(|i| (format!("h{i}"), "v".to_string()))
        .collect();
    let headers: Vec<(&str, &str)> = headers_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let err = IncomingEvent::try_new(b"ok", &headers).unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[test]
fn default_limits_match_documented_constants() {
    // Regression guard: the public constants should remain stable.
    // If intentionally bumped, update this test and the doc comment.
    assert_eq!(DEFAULT_MAX_BODY_BYTES, 1024 * 1024);
    assert_eq!(MAX_HEADER_COUNT, 128);
}
