use super::*;

#[test]
fn retryable_error_is_retryable() {
    let err = ActionError::retryable("connection reset");
    assert!(err.is_retryable());
    assert!(!err.is_fatal());
    assert!(err.backoff_hint().is_none());
}

#[test]
fn retryable_with_backoff_carries_hint() {
    let err = ActionError::retryable_with_backoff("rate limited", Duration::from_secs(5));
    assert!(err.is_retryable());
    assert_eq!(err.backoff_hint(), Some(Duration::from_secs(5)));
}

#[test]
fn retryable_with_partial_carries_output() {
    let partial = serde_json::json!({"processed": 3});
    let err = ActionError::retryable_with_partial("partial failure", partial.clone());
    assert!(err.is_retryable());
    assert_eq!(err.partial_output(), Some(&partial));
}

#[test]
fn fatal_error_is_not_retryable() {
    let err = ActionError::fatal("invalid credentials");
    assert!(err.is_fatal());
    assert!(!err.is_retryable());
}

#[test]
fn fatal_with_details() {
    let details = serde_json::json!({"field": "password"});
    let err = ActionError::fatal_with_details("auth failed", details.clone());
    match &err {
        ActionError::Fatal { details: d, .. } => assert_eq!(d, &Some(details)),
        _ => panic!("expected Fatal"),
    }
}

#[test]
fn validation_error_is_fatal() {
    let err = ActionError::validation("email", ValidationReason::MissingField, None::<String>);
    assert!(err.is_fatal());
    assert!(!err.is_retryable());
}

#[test]
fn capability_violation_is_fatal() {
    let err = ActionError::CapabilityViolation {
        capability: "Network".into(),
        action_id: "custom.action".into(),
    };
    assert!(err.is_fatal());
    assert!(!err.is_retryable());
}

#[test]
fn cancelled_is_neither_retryable_nor_fatal() {
    let err = ActionError::Cancelled;
    assert!(!err.is_retryable());
    // Cancelled is special — not retryable, not "fatal" in the business sense
    assert!(!err.is_fatal());
}

#[test]
fn data_limit_exceeded_is_fatal() {
    let err = ActionError::DataLimitExceeded {
        limit_bytes: 1_000_000,
        actual_bytes: 5_000_000,
    };
    assert!(err.is_fatal());
}

#[test]
fn retry_hint_code_serializes_to_string() {
    let hint = RetryHintCode::RateLimited;
    let json = serde_json::to_string(&hint).unwrap();
    assert_eq!(json, "\"RateLimited\"");
}

#[test]
fn retry_hint_code_deserializes_from_string() {
    let hint: RetryHintCode = serde_json::from_str("\"AuthExpired\"").unwrap();
    assert_eq!(hint, RetryHintCode::AuthExpired);
}

#[test]
fn retry_hint_code_is_copy() {
    let hint = RetryHintCode::RateLimited;
    let copy = hint;
    assert_eq!(hint, copy); // both still valid — Copy
}

#[test]
fn retry_hint_code_debug_format() {
    assert_eq!(
        format!("{:?}", RetryHintCode::UpstreamTimeout),
        "UpstreamTimeout"
    );
}

#[test]
fn display_formatting() {
    let err = ActionError::retryable("timeout");
    assert_eq!(err.to_string(), "retryable action failure");
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("timeout".to_owned())
    );

    let err = ActionError::fatal("bad schema");
    assert_eq!(err.to_string(), "fatal action failure");
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("bad schema".to_owned())
    );

    let err = ActionError::validation("email", ValidationReason::MissingField, None::<String>);
    assert_eq!(err.to_string(), "validation (missing_field): field `email`");

    let err = ActionError::validation(
        "body",
        ValidationReason::MalformedJson,
        Some("expected object"),
    );
    assert_eq!(
        err.to_string(),
        "validation (malformed_json): field `body` — expected object"
    );

    let err = ActionError::Cancelled;
    assert_eq!(err.to_string(), "cancelled");
}

#[test]
fn retryable_with_hint_attaches_hint() {
    let err = ActionError::retryable_with_hint("rate limited", RetryHintCode::RateLimited);
    assert_eq!(err.retry_hint_code(), Some(RetryHintCode::RateLimited));
    assert!(err.is_retryable());
}

#[test]
fn fatal_with_hint_attaches_hint() {
    let err = ActionError::fatal_with_hint("expired", RetryHintCode::AuthExpired);
    assert_eq!(err.retry_hint_code(), Some(RetryHintCode::AuthExpired));
    assert!(err.is_fatal());
}

#[test]
fn retryable_from_preserves_error_chain() {
    let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
    let err = ActionError::retryable_from(io_err);
    assert_eq!(err.to_string(), "retryable action failure");
    assert!(err.is_retryable());
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("timeout".to_owned())
    );
    assert_eq!(
        std::error::Error::source(&err)
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::TimedOut)
    );
}

#[test]
fn fatal_from_preserves_typed_source_after_clone() {
    let original = ActionError::fatal_from(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    let cloned = original.clone();
    for error in [original, cloned] {
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("denied".to_owned())
        );
        assert_eq!(
            std::error::Error::source(&error)
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }
}

#[test]
fn clone_preserves_error() {
    let err = ActionError::retryable("test");
    let cloned = err.clone();
    assert_eq!(err.to_string(), cloned.to_string());
}

#[test]
fn retry_hint_code_is_none_when_not_supplied() {
    let err = ActionError::retryable("no hint");
    assert!(err.retry_hint_code().is_none());
}

#[test]
fn retry_hint_code_is_none_for_non_retryable_fatal_variants() {
    // Variants other than Retryable/Fatal never carry a user hint —
    // use Classify::code() for the framework tag instead.
    assert!(
        ActionError::validation("x", ValidationReason::Other, None::<String>)
            .retry_hint_code()
            .is_none()
    );
    assert!(ActionError::Cancelled.retry_hint_code().is_none());
    assert!(
        ActionError::DataLimitExceeded {
            limit_bytes: 1,
            actual_bytes: 2,
        }
        .retry_hint_code()
        .is_none()
    );
}

// ── ActionErrorExt ──────────────────────────────────────────────────────

#[test]
fn ext_retryable_converts_io_error() {
    let result: Result<(), std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "connection refused",
    ));
    let err = result.retryable().unwrap_err();
    assert!(err.is_retryable());
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("connection refused".to_owned())
    );
}

#[test]
fn ext_fatal_converts_io_error() {
    let result: Result<(), std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "corrupt",
    ));
    let err = result.fatal().unwrap_err();
    assert!(err.is_fatal());
}

#[test]
fn ext_retryable_with_hint_sets_hint() {
    let result: Result<i32, std::io::Error> = Err(std::io::Error::other("rate limited"));
    let err = result
        .retryable_with_hint(RetryHintCode::RateLimited)
        .unwrap_err();
    assert_eq!(err.retry_hint_code(), Some(RetryHintCode::RateLimited));
    assert!(err.is_retryable());
}

#[test]
fn ext_fatal_with_hint_sets_hint() {
    let result: Result<i32, std::io::Error> = Err(std::io::Error::other("expired"));
    let err = result
        .fatal_with_hint(RetryHintCode::AuthExpired)
        .unwrap_err();
    assert_eq!(err.retry_hint_code(), Some(RetryHintCode::AuthExpired));
    assert!(err.is_fatal());
}

#[test]
fn ext_ok_passes_through() {
    let result: Result<i32, std::io::Error> = Ok(42);
    assert_eq!(result.retryable().unwrap(), 42);
}

#[test]
fn ext_chaining_preserves_error_chain() {
    fn do_io() -> Result<Vec<u8>, std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
    }
    fn do_work() -> Result<String, ActionError> {
        let _data = do_io().retryable()?;
        Ok("ok".into())
    }
    let err = do_work().unwrap_err();
    assert!(err.is_retryable());
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("missing".to_owned())
    );
}

// ── ValidationReason + structured Validation (L7) ──────────────────────

#[test]
fn core_resource_unavailable_retryable_becomes_action_retryable() {
    let core = nebula_core::CoreError::resource_unavailable(
        "postgres",
        "pool exhausted",
        true,
        Some(Duration::from_millis(50)),
    );
    let action: ActionError = core.into();
    assert!(matches!(action, ActionError::Retryable { .. }));
    assert_eq!(action.backoff_hint(), Some(Duration::from_millis(50)));
}

#[test]
fn validation_reason_as_str_stable() {
    assert_eq!(ValidationReason::MissingField.as_str(), "missing_field");
    assert_eq!(ValidationReason::WrongType.as_str(), "wrong_type");
    assert_eq!(ValidationReason::OutOfRange.as_str(), "out_of_range");
    assert_eq!(ValidationReason::MalformedJson.as_str(), "malformed_json");
    assert_eq!(
        ValidationReason::StateDeserialization.as_str(),
        "state_deserialization"
    );
    assert_eq!(ValidationReason::Other.as_str(), "other");
}

#[test]
fn validation_sanitizes_newlines_and_ansi() {
    // Log injection test: an attacker-supplied string embeds newlines
    // and fake audit entries that would otherwise show up verbatim in
    // JSON log sinks. Sanitize escapes them as \u000a / \u000d so the
    // actual line break never survives.
    let err = ActionError::validation(
        "body",
        ValidationReason::MalformedJson,
        Some("line1\nline2\r\nfake audit entry"),
    );
    let msg = err.to_string();
    assert!(!msg.contains('\n'), "newline must be escaped: {msg}");
    assert!(!msg.contains('\r'), "CR must be escaped: {msg}");
    assert!(msg.contains("\\u000a"), "expected escaped LF in {msg}");
    assert!(msg.contains("\\u000d"), "expected escaped CR in {msg}");
}

#[test]
fn validation_sanitizes_null_byte() {
    let err = ActionError::validation("x", ValidationReason::Other, Some("null\0here"));
    let msg = err.to_string();
    assert!(!msg.contains('\0'));
    assert!(msg.contains("\\u0000"));
}

#[test]
fn validation_truncates_long_detail() {
    let huge = "A".repeat(10_000);
    let err = ActionError::validation("body", ValidationReason::Other, Some(huge));
    let ActionError::Validation { detail, .. } = &err else {
        panic!("expected Validation variant");
    };
    let d = detail.as_ref().expect("detail present");
    // Budget + ellipsis marker ≤ MAX + a few bytes for '…' UTF-8.
    assert!(
        d.len() <= MAX_VALIDATION_DETAIL + 4,
        "detail len {} > budget",
        d.len()
    );
    assert!(
        d.ends_with('…'),
        "truncated detail must end with ellipsis: {d}"
    );
}

#[test]
fn validation_no_detail_still_useful() {
    let err = ActionError::validation("email", ValidationReason::MissingField, None::<String>);
    let s = err.to_string();
    assert!(s.contains("missing_field"), "{s}");
    assert!(s.contains("email"), "{s}");
}

#[test]
fn validation_structured_fields_preserved() {
    let err = ActionError::validation("email", ValidationReason::WrongType, Some("got number"));
    let ActionError::Validation {
        field,
        reason,
        detail,
    } = &err
    else {
        panic!("expected Validation variant");
    };
    assert_eq!(*field, "email");
    assert_eq!(*reason, ValidationReason::WrongType);
    assert_eq!(detail.as_deref(), Some("got number"));
}

// ── CredentialRefreshFailed ─────────────────────────────────────────────

#[test]
fn credential_refresh_failed_is_retryable() {
    let err = ActionError::credential_refresh_failed(
        "http.fetch",
        std::io::Error::new(std::io::ErrorKind::ConnectionReset, "store down"),
    );
    assert!(err.is_retryable());
    assert!(!err.is_fatal());
}

#[test]
fn credential_refresh_failed_display_includes_source_and_key() {
    let err =
        ActionError::credential_refresh_failed("http.fetch", std::io::Error::other("store down"));
    let msg = err.to_string();
    assert!(msg.contains("http.fetch"), "{msg}");
    assert!(!msg.contains("store down"), "{msg}");
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("store down".to_owned())
    );
}

#[test]
fn credential_refresh_failed_classify_code_is_stable() {
    use nebula_error::Classify;
    let err = ActionError::credential_refresh_failed("http.fetch", std::io::Error::other("boom"));
    assert_eq!(err.code().as_str(), "ACTION:CREDENTIAL_REFRESH_FAILED");
}

#[test]
fn credential_refresh_failed_is_clone() {
    let err = ActionError::credential_refresh_failed("http.fetch", std::io::Error::other("boom"));
    // The whole point of wrapping `source` in `Arc` is that the
    // variant remains `Clone`-compatible with the rest of
    // `ActionError`.
    let cloned = err.clone();
    assert_eq!(err.to_string(), cloned.to_string());
}

#[test]
fn validation_reason_serializes_to_variant_name() {
    let json = serde_json::to_string(&ValidationReason::MalformedJson).unwrap();
    assert_eq!(json, "\"MalformedJson\"");
    let parsed: ValidationReason = serde_json::from_str("\"MissingField\"").unwrap();
    assert_eq!(parsed, ValidationReason::MissingField);
}
