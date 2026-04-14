//! Integration tests for `WebhookRequest` body-size and header-count limits.
//!
//! Cover the DoS-hardening construction path:
//! - oversized body → `DataLimitExceeded`
//! - header count above cap → `Validation`
//! - custom limits via `webhook_request_for_test_with_limits`
//! - case-insensitive header lookup
//! - body accessor invariants

use nebula_action::{
    ActionError, DEFAULT_MAX_BODY_BYTES, MAX_HEADER_COUNT,
    webhook::{webhook_request_for_test, webhook_request_for_test_with_limits},
};

#[test]
fn try_new_accepts_empty_body() {
    assert!(webhook_request_for_test(&[], &[]).is_ok());
}

#[test]
fn try_new_accepts_exact_limit_body() {
    let body = vec![0u8; DEFAULT_MAX_BODY_BYTES];
    assert!(webhook_request_for_test(&body, &[]).is_ok());
}

#[test]
fn try_new_rejects_oversized_body() {
    let body = vec![0u8; DEFAULT_MAX_BODY_BYTES + 1];
    let err = webhook_request_for_test(&body, &[]).unwrap_err();
    match err {
        ActionError::DataLimitExceeded {
            limit_bytes,
            actual_bytes,
        } => {
            assert_eq!(limit_bytes, DEFAULT_MAX_BODY_BYTES as u64);
            assert_eq!(actual_bytes, (DEFAULT_MAX_BODY_BYTES + 1) as u64);
        },
        other => panic!("expected DataLimitExceeded, got {other:?}"),
    }
}

#[test]
fn try_new_with_limits_custom_cap_accepts_under() {
    let body = vec![0u8; 2048];
    assert!(webhook_request_for_test_with_limits(&body, &[], 4096, 16).is_ok());
}

#[test]
fn try_new_with_limits_custom_cap_rejects_over() {
    let body = vec![0u8; 2048];
    let err = webhook_request_for_test_with_limits(&body, &[], 1024, 16).unwrap_err();
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
    assert!(webhook_request_for_test(b"", &headers).is_ok());
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
    let err = webhook_request_for_test(b"", &headers).unwrap_err();
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
        },
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn try_new_with_limits_custom_header_cap() {
    let err =
        webhook_request_for_test_with_limits(b"", &[("a", "1"), ("b", "2"), ("c", "3")], 1024, 2)
            .unwrap_err();
    assert!(matches!(err, ActionError::Validation { .. }));
}

#[test]
fn header_lookup_is_case_insensitive() {
    let req = webhook_request_for_test(b"", &[("x-custom", "v1"), ("x-other", "v2")]).unwrap();
    assert_eq!(req.header_str("x-custom"), Some("v1"));
    assert_eq!(req.header_str("X-Custom"), Some("v1"));
    assert_eq!(req.header_str("X-OTHER"), Some("v2"));
    assert_eq!(req.header_str("missing"), None);
}

#[test]
fn body_accessor_returns_canonical_bytes() {
    let req = webhook_request_for_test(b"hello", &[]).unwrap();
    assert_eq!(req.body(), b"hello");
    assert_eq!(req.body_str(), Some("hello"));
}

#[test]
fn default_limits_match_documented_constants() {
    assert_eq!(DEFAULT_MAX_BODY_BYTES, 1024 * 1024);
    // H8 bump: 128 → 256 to accommodate CF+NGINX+service-mesh stacks.
    assert_eq!(MAX_HEADER_COUNT, 256);
}

// ── H5: body_json_bounded depth cap ──────────────────────────────────────

#[test]
fn body_json_bounded_accepts_reasonable_nesting() {
    let body = br#"{"a":{"b":{"c":{"d":1}}}}"#;
    let req = nebula_action::webhook::webhook_request_for_test(body, &[]).unwrap();
    let v: serde_json::Value = req.body_json_bounded(64).expect("depth 4 fits under 64");
    assert_eq!(v["a"]["b"]["c"]["d"], 1);
}

#[test]
fn body_json_bounded_rejects_deep_nesting() {
    // Build {"a":{"a":{"a": ... }}} with 100 levels.
    let mut body = String::new();
    for _ in 0..100 {
        body.push_str("{\"a\":");
    }
    body.push('1');
    for _ in 0..100 {
        body.push('}');
    }
    let req = nebula_action::webhook::webhook_request_for_test(body.as_bytes(), &[]).unwrap();
    let err = req
        .body_json_bounded::<serde_json::Value>(64)
        .expect_err("100 levels must exceed max_depth=64");
    assert!(
        err.to_string().contains("max depth"),
        "error should mention depth: {err}"
    );
}

#[test]
fn body_json_bounded_handles_arrays() {
    // Nested arrays should count toward depth the same as objects.
    let body = b"[[[[[[[[1]]]]]]]]";
    let req = nebula_action::webhook::webhook_request_for_test(body, &[]).unwrap();
    assert!(req.body_json_bounded::<serde_json::Value>(10).is_ok());
    assert!(req.body_json_bounded::<serde_json::Value>(5).is_err());
}

#[test]
fn body_json_bounded_ignores_braces_in_strings() {
    // A brace inside a string literal should NOT count toward depth.
    let body = br#"{"comment":"this has {{{ braces }}} inside"}"#;
    let req = nebula_action::webhook::webhook_request_for_test(body, &[]).unwrap();
    let v: serde_json::Value = req
        .body_json_bounded(2)
        .expect("string-bracketed braces must not inflate depth");
    assert_eq!(v["comment"], "this has {{{ braces }}} inside");
}

#[test]
fn body_json_bounded_handles_escaped_quotes() {
    // An escaped quote inside a string must not end string parsing
    // prematurely — otherwise a { inside what looks like a string
    // would count as real depth.
    let body = br#"{"quote":"she said \"hello\"","next":1}"#;
    let req = nebula_action::webhook::webhook_request_for_test(body, &[]).unwrap();
    let v: serde_json::Value = req.body_json_bounded(8).unwrap();
    assert_eq!(v["quote"], "she said \"hello\"");
    assert_eq!(v["next"], 1);
}
