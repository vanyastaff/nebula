//! Public contract tests for shared policy context behavior.
//!
//! Module unit tests cover many individual branches. These integration tests
//! pin the behavior that downstream Nebula crates rely on through the public
//! API when policies are composed under one cancellation/deadline context.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nebula_resilience::{
    CallError, CancellationContext, Gate, PolicyContext, RecordingSink, ResilienceEventKind,
    fallback::{FallbackOperation, ValueFallback},
    load_shed_with_policy_context, timeout_with_policy_context,
    timeout_with_policy_context_and_sink,
};

#[tokio::test]
async fn timeout_context_cancellation_wins_without_polling_future() {
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let polled = Arc::new(AtomicBool::new(false));
    let future_polled = Arc::clone(&polled);

    let result: Result<(), CallError<()>> =
        timeout_with_policy_context(&context, Duration::from_secs(1), async move {
            future_polled.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;

    assert!(matches!(
        result,
        Err(CallError::Cancelled {
            reason: Some(reason)
        }) if reason == "shutdown"
    ));
    assert!(!polled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn timeout_local_timeout_emits_event_when_it_wins() {
    let context = PolicyContext::with_timeout(Duration::from_secs(1));
    let sink = RecordingSink::new();

    let result: Result<(), CallError<()>> = timeout_with_policy_context_and_sink(
        &context,
        Duration::from_millis(1),
        async {
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok(())
        },
        &sink,
    )
    .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
    assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);
}

#[tokio::test]
async fn load_shed_context_cancellation_skips_predicate() {
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let predicate_called = Arc::new(AtomicBool::new(false));
    let predicate_observed = Arc::clone(&predicate_called);

    let result: Result<u32, CallError<()>> = load_shed_with_policy_context(
        &context,
        move || {
            predicate_observed.store(true, Ordering::SeqCst);
            true
        },
        || async { Ok(42) },
    )
    .await;

    assert!(matches!(
        result,
        Err(CallError::Cancelled {
            reason: Some(reason)
        }) if reason == "shutdown"
    ));
    assert!(!predicate_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn load_shed_context_deadline_bounds_operation() {
    let context = PolicyContext::with_timeout(Duration::from_millis(1));

    let result: Result<u32, CallError<()>> = load_shed_with_policy_context(
        &context,
        || false,
        || async {
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok(42)
        },
    )
    .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
}

#[tokio::test]
async fn fallback_context_cancellation_emits_no_fallback_events() {
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let sink = RecordingSink::new();
    let operation: FallbackOperation<u32, ()> =
        FallbackOperation::new(Arc::new(ValueFallback::new(99))).with_sink(sink.clone());

    let result = operation
        .call_with_policy_context(&context, || async {
            Err(CallError::Timeout(Duration::from_millis(1)))
        })
        .await;

    assert!(matches!(
        result,
        Err(CallError::Cancelled {
            reason: Some(reason)
        }) if reason == "shutdown"
    ));
    assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 0);
    assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 0);
    assert_eq!(sink.count(ResilienceEventKind::FallbackFailed), 0);
}

#[tokio::test]
async fn gate_close_with_timeout_keeps_gate_closed_until_later_drain() {
    let gate = Gate::new();
    let guard = gate.enter().expect("gate should start open");

    let error = gate
        .close_with_timeout(Duration::from_millis(1))
        .await
        .expect_err("held guard should block graceful drain");

    assert_eq!(error.active_guards, 1);
    assert!(gate.is_closed());
    assert!(gate.enter().is_err());

    drop(guard);
    gate.close().await;

    assert_eq!(gate.active_count(), 0);
    assert!(gate.is_closed());
}
