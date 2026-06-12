use std::time::Duration;

use nebula_resource::{ClassifyError, Error, ErrorKind};

#[derive(Debug, thiserror::Error, ClassifyError)]
enum TestError {
    #[error("auth failed: {0}")]
    #[classify(permanent)]
    Auth(String),

    #[error("connection failed")]
    #[classify(transient)]
    Connect(#[from] std::io::Error),

    #[error("rate limited")]
    #[classify(exhausted, retry_after = "30s")]
    RateLimit,

    #[error("quota depleted")]
    #[classify(exhausted)]
    QuotaDepleted,

    #[error("pool full")]
    #[classify(backpressure)]
    PoolFull,

    #[error("cancelled")]
    #[classify(cancelled)]
    Cancelled,
}

#[test]
fn permanent_variant_maps_correctly() {
    let err: Error = TestError::Auth("bad password".into()).into();
    assert_eq!(*err.kind(), ErrorKind::Permanent);
    assert!(!err.is_retryable());
    assert!(err.to_string().contains("auth failed"));
}

#[test]
fn transient_variant_maps_correctly() {
    let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
    let err: Error = TestError::Connect(io_err).into();
    assert_eq!(*err.kind(), ErrorKind::Transient);
    assert!(err.is_retryable());
}

#[test]
fn exhausted_with_retry_after() {
    let err: Error = TestError::RateLimit.into();
    assert_eq!(
        *err.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(30))
        }
    );
    assert!(err.is_retryable());
    assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
}

#[test]
fn exhausted_without_retry_after() {
    let err: Error = TestError::QuotaDepleted.into();
    assert_eq!(*err.kind(), ErrorKind::Exhausted { retry_after: None });
    assert!(err.is_retryable());
    assert_eq!(err.retry_after(), None);
}

#[test]
fn backpressure_variant_maps_correctly() {
    let err: Error = TestError::PoolFull.into();
    assert_eq!(*err.kind(), ErrorKind::Backpressure);
    assert!(err.is_retryable());
}

#[test]
fn cancelled_variant_maps_correctly() {
    let err: Error = TestError::Cancelled.into();
    assert_eq!(*err.kind(), ErrorKind::Cancelled);
    assert!(!err.is_retryable());
}

/// Verify that named-field variants work correctly.
#[derive(Debug, thiserror::Error, ClassifyError)]
enum NamedFieldError {
    #[error("timeout after {duration_ms}ms")]
    #[classify(transient)]
    Timeout { duration_ms: u64 },

    #[error("invalid config: {reason}")]
    #[classify(permanent)]
    InvalidConfig { reason: String },
}

#[test]
fn named_field_variant_works() {
    let err: Error = NamedFieldError::Timeout { duration_ms: 5000 }.into();
    assert_eq!(*err.kind(), ErrorKind::Transient);
    assert!(err.to_string().contains("5000"));
}

#[test]
fn named_field_permanent_works() {
    let err: Error = NamedFieldError::InvalidConfig {
        reason: "missing host".into(),
    }
    .into();
    assert_eq!(*err.kind(), ErrorKind::Permanent);
    assert!(err.to_string().contains("missing host"));
}

/// Duration parsing: minutes and hours.
#[derive(Debug, thiserror::Error, ClassifyError)]
enum DurationError {
    #[error("rate limit 5m")]
    #[classify(exhausted, retry_after = "5m")]
    FiveMinutes,

    #[error("rate limit 1h")]
    #[classify(exhausted, retry_after = "1h")]
    OneHour,

    #[error("rate limit 500ms")]
    #[classify(exhausted, retry_after = "500ms")]
    HalfSecond,
}

#[test]
fn duration_minutes() {
    let err: Error = DurationError::FiveMinutes.into();
    assert_eq!(err.retry_after(), Some(Duration::from_mins(5)));
}

#[test]
fn duration_hours() {
    let err: Error = DurationError::OneHour.into();
    assert_eq!(err.retry_after(), Some(Duration::from_hours(1)));
}

#[test]
fn duration_milliseconds() {
    let err: Error = DurationError::HalfSecond.into();
    assert_eq!(err.retry_after(), Some(Duration::from_millis(500)));
}

/// Runtime `retry_after` read from a variant field — both a tuple index
/// (`.0`) and a named field. Regression guard: the emitted `From` must read
/// the `Duration` field through a borrow and only move the error into
/// `with_source` afterwards, or it fails to compile (E0505). No prior test
/// exercised these forms, so the broken emission compiled-but-was-dead.
#[derive(Debug, thiserror::Error, ClassifyError)]
enum RuntimeRetryError {
    #[error("throttled, retry in {0:?}")]
    #[classify(exhausted, retry_after = .0)]
    Throttled(Duration),

    #[error("quota exceeded, wait {wait:?}")]
    #[classify(exhausted, retry_after = wait)]
    QuotaExceeded { wait: Duration },
}

#[test]
fn runtime_retry_after_tuple_field() {
    let err: Error = RuntimeRetryError::Throttled(Duration::from_secs(7)).into();
    assert_eq!(
        *err.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_secs(7))
        }
    );
    assert_eq!(err.retry_after(), Some(Duration::from_secs(7)));
    // The original error is preserved as the source chain.
    assert!(std::error::Error::source(&err).is_some());
}

#[test]
fn runtime_retry_after_named_field() {
    let err: Error = RuntimeRetryError::QuotaExceeded {
        wait: Duration::from_millis(250),
    }
    .into();
    assert_eq!(
        *err.kind(),
        ErrorKind::Exhausted {
            retry_after: Some(Duration::from_millis(250))
        }
    );
    assert_eq!(err.retry_after(), Some(Duration::from_millis(250)));
    assert!(std::error::Error::source(&err).is_some());
}
