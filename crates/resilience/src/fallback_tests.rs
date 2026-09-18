use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::*;
use crate::{CallError, CancellationContext, PolicyContext, RecordingSink, ResilienceEventKind};

fn timeout_error() -> CallError<&'static str> {
    CallError::Timeout(Duration::from_secs(1))
}

fn cancelled_error() -> CallError<&'static str> {
    CallError::cancelled()
}

struct CountingFallback {
    calls: Arc<AtomicUsize>,
}

impl FallbackStrategy<u32, &'static str> for CountingFallback {
    fn recover<'a>(
        &'a self,
        _error: CallError<&'static str>,
    ) -> Pin<Box<dyn Future<Output = Result<u32, CallError<&'static str>>> + Send + 'a>> {
        Box::pin(async { Ok(7) })
    }

    fn should_fallback(&self, _error: &CallError<&'static str>) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        true
    }
}

// -----------------------------------------------------------------------
// ValueFallback
// -----------------------------------------------------------------------

#[tokio::test]
async fn value_fallback_returns_configured_value() {
    let fb = ValueFallback::new(42u32);
    let result = fb.fallback(timeout_error()).await;
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn value_fallback_should_fallback_true_for_timeout() {
    let fb = ValueFallback::<u32>::new(0u32);
    assert!(fb.should_fallback(&timeout_error()));
}

#[test]
fn value_fallback_declines_cancellation_and_overload_by_default() {
    let fb = ValueFallback::<u32>::new(0u32);
    let load_shed: CallError<&str> = CallError::LoadShed;
    let rate_limited: CallError<&str> = CallError::rate_limited();
    let bulkhead: CallError<&str> = CallError::BulkheadFull;

    assert!(!fb.should_fallback(&cancelled_error()));
    assert!(!fb.should_fallback(&load_shed));
    assert!(!fb.should_fallback(&rate_limited));
    assert!(!fb.should_fallback(&bulkhead));
}

#[tokio::test]
async fn value_fallback_direct_call_declines_cancellation() {
    let fb = ValueFallback::<u32>::new(0u32);
    let result = fb.fallback(cancelled_error()).await;
    assert!(matches!(result, Err(CallError::Cancelled { .. })));
}

// -----------------------------------------------------------------------
// CacheFallback
// -----------------------------------------------------------------------

#[tokio::test]
async fn cache_fallback_returns_error_when_empty() {
    let fb: CacheFallback<String> = CacheFallback::new();
    let result: Result<String, CallError<&str>> = fb
        .fallback(CallError::Timeout(Duration::from_secs(1)))
        .await;
    assert!(matches!(result, Err(CallError::Timeout(_))));
}

#[tokio::test]
async fn cache_fallback_returns_cached_value() {
    let fb = CacheFallback::new();
    fb.update("hello".to_string()).await;
    let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
    assert_eq!(result.unwrap(), "hello");
}

#[tokio::test]
async fn cache_fallback_expires_when_ttl_exceeded() {
    let fb = CacheFallback::new().with_ttl(Duration::from_millis(1));
    fb.update("stale".to_string()).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
    assert!(matches!(result, Err(CallError::Timeout(_))));
}

#[tokio::test]
async fn cache_fallback_stale_if_error_serves_expired_value() {
    let fb = CacheFallback::new()
        .with_ttl(Duration::from_millis(1))
        .with_stale_if_error(true);
    fb.update("stale".to_string()).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
    assert_eq!(result.unwrap(), "stale");
}

// -----------------------------------------------------------------------
// ChainFallback
// -----------------------------------------------------------------------

#[tokio::test]
async fn chain_fallback_tries_in_order_and_returns_first_success() {
    let first: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(CacheFallback::new());
    let second: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(99u32));

    let chain = ChainFallback::new().then(first).then(second);
    let result = chain.fallback(timeout_error()).await;
    assert_eq!(result.unwrap(), 99);
}

#[tokio::test]
async fn chain_fallback_returns_last_error_when_all_fail() {
    let failing: Arc<dyn FallbackStrategy<u32, &str>> =
        Arc::new(FunctionFallback::new(|_err| async {
            Err(CallError::cancelled_with("fail"))
        }));
    let chain = ChainFallback::new()
        .then(Arc::clone(&failing))
        .then(Arc::clone(&failing));
    let result = chain.fallback(timeout_error()).await;
    assert!(matches!(
        result,
        Err(CallError::FallbackFailedWithContext { .. })
    ));
}

#[tokio::test]
async fn function_fallback_failure_preserves_primary_context() {
    let fallback = FunctionFallback::new(|_err: CallError<()>| async {
        Err::<u32, _>(CallError::cancelled_with("fallback unavailable"))
    });

    let result = fallback
        .fallback(CallError::Operation("primary failed"))
        .await;
    let err = result.unwrap_err();
    let (primary, fallback) = err.fallback_context().unwrap();

    assert!(matches!(primary, CallError::Operation("primary failed")));
    assert!(matches!(fallback, CallError::Cancelled { .. }));
}

// -----------------------------------------------------------------------
// PriorityFallback / CallErrorKind
// -----------------------------------------------------------------------

#[test]
fn error_kind_from_timeout() {
    assert_eq!(timeout_error().kind(), CallErrorKind::Timeout);
}

#[test]
fn error_kind_from_cancelled() {
    assert_eq!(cancelled_error().kind(), CallErrorKind::Cancelled);
}

#[tokio::test]
async fn priority_fallback_dispatches_to_matching_kind() {
    let timeout_fb: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(1u32));
    let default_fb: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(0u32));

    let pf = PriorityFallback::new()
        .register(CallErrorKind::Timeout, timeout_fb)
        .with_default(default_fb);

    // Timeout → registered handler
    assert_eq!(pf.fallback(timeout_error()).await.unwrap(), 1);
    // Fallbackable error without a specific handler → default
    assert_eq!(pf.fallback(CallError::Operation("boom")).await.unwrap(), 0);
    // Cancellation is not recovered by the default fallback.
    assert!(matches!(
        pf.fallback(cancelled_error()).await,
        Err(CallError::Cancelled { .. })
    ));
}

#[tokio::test]
async fn priority_fallback_returns_error_when_no_match_and_no_default() {
    let pf: PriorityFallback<u32, &str> = PriorityFallback::new();
    let result = pf.fallback(timeout_error()).await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// FallbackExecutor
// -----------------------------------------------------------------------

#[tokio::test]
async fn fallback_operation_returns_primary_result_on_success() {
    let op: FallbackExecutor<u32, &str> = FallbackExecutor::new(Arc::new(ValueFallback::new(0u32)));
    let result = op.call(|| async { Ok(42u32) }).await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn fallback_operation_invokes_fallback_on_error() {
    let op: FallbackExecutor<u32, &str> =
        FallbackExecutor::new(Arc::new(ValueFallback::new(99u32)));
    let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;
    assert_eq!(result.unwrap(), 99);
}

#[tokio::test]
async fn fallback_operation_emits_standalone_lifecycle_events() {
    let sink = RecordingSink::new();
    let op: FallbackExecutor<u32, &str> =
        FallbackExecutor::new(Arc::new(ValueFallback::new(99u32))).with_sink(sink.clone());

    let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;

    assert_eq!(result.unwrap(), 99);
    assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
    assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 1);
    assert_eq!(sink.count(ResilienceEventKind::FallbackFailed), 0);
}

#[tokio::test]
async fn fallback_operation_emits_failure_event_on_fallback_failure() {
    let sink = RecordingSink::new();
    let fallback = FunctionFallback::new(|_err: CallError<()>| async {
        Err::<u32, _>(CallError::fallback_failed_with("cache unavailable"))
    });
    let op: FallbackExecutor<u32, &str> =
        FallbackExecutor::new(Arc::new(fallback)).with_sink(sink.clone());

    let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;

    assert!(matches!(
        result,
        Err(CallError::FallbackFailedWithContext { .. })
    ));
    assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
    assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 0);
    assert_eq!(sink.count(ResilienceEventKind::FallbackFailed), 1);
    assert!(sink.events().iter().any(|event| matches!(
        event,
        ResilienceEvent::FallbackFailed {
            primary_error: CallErrorKind::Timeout,
            fallback_error: CallErrorKind::FallbackFailed,
        }
    )));
}

#[tokio::test]
async fn fallback_operation_evaluates_should_fallback_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let op = FallbackExecutor::new(Arc::new(CountingFallback {
        calls: Arc::clone(&calls),
    }));

    let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;

    assert_eq!(result.unwrap(), 7);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fallback_operation_context_cancellation_skips_fallback() {
    let op: FallbackExecutor<u32, &str> =
        FallbackExecutor::new(Arc::new(ValueFallback::new(99u32)));
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let result = op
        .call_with_policy_context(&context, || async { Ok::<u32, CallError<&str>>(42) })
        .await;

    assert!(matches!(result, Err(CallError::Cancelled { .. })));
}

#[tokio::test]
async fn fallback_operation_preserves_non_context_cancellation_reason() {
    let op: FallbackExecutor<u32, &str> =
        FallbackExecutor::new(Arc::new(ValueFallback::new(99u32)));
    let context = PolicyContext::empty();

    let result = op
        .call_with_policy_context(&context, || async {
            Err::<u32, _>(CallError::cancelled_with("primary stopped itself"))
        })
        .await;

    assert!(matches!(
        result,
        Err(CallError::Cancelled {
            reason: Some(reason)
        }) if reason == "primary stopped itself"
    ));
}

#[tokio::test]
async fn fallback_operation_context_deadline_bounds_fallback() {
    let fallback = FunctionFallback::new(|_err: CallError<()>| async {
        tokio::time::sleep(Duration::from_mins(1)).await;
        Ok::<u32, CallError<()>>(99)
    });
    let op: FallbackExecutor<u32, &str> = FallbackExecutor::new(Arc::new(fallback));
    let context = PolicyContext::with_timeout(Duration::from_millis(1));

    let result = op
        .call_with_policy_context(&context, || async { Err::<u32, _>(timeout_error()) })
        .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
}
