use std::{
    fmt,
    future::ready,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_error::{Classify, ErrorCategory, ErrorCode, RetryHint, codes};

use super::*;
use crate::{
    CallContext, CallError, CancellationContext, CircuitBreaker, RecordingSink,
    ResilienceEventKind, retry::BackoffConfig,
};

#[derive(Debug, Clone, Copy)]
struct RetryAfterErr;

impl fmt::Display for RetryAfterErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("retry after")
    }
}

impl Classify for RetryAfterErr {
    fn category(&self) -> ErrorCategory {
        ErrorCategory::RateLimit
    }

    fn code(&self) -> ErrorCode {
        codes::INTERNAL
    }

    fn retry_hint(&self) -> Option<RetryHint> {
        Some(RetryHint::after(Duration::from_millis(25)))
    }
}

async fn delay_first_two_attempts(attempt: u32) {
    if attempt < 2 {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn fail_transient_after_count(
    seen: Arc<AtomicU32>,
) -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> {
    Box::pin(async move {
        seen.fetch_add(1, Ordering::SeqCst);
        Err("transient")
    })
}

fn reject_first_rate_limit_check(
    checks: &AtomicU32,
    retry_after: Duration,
) -> Result<(), CallError<()>> {
    if checks.fetch_add(1, Ordering::SeqCst) == 0 {
        Err(CallError::rate_limited_after(retry_after))
    } else {
        Ok(())
    }
}

fn always_reject_rate_limit_check(checks: &AtomicU32) -> Result<(), CallError<()>> {
    checks.fetch_add(1, Ordering::SeqCst);
    Err(CallError::rate_limited())
}

fn fail_once_with_retry_hint(attempts: &AtomicU32) -> Result<u32, RetryAfterErr> {
    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
        return Err(RetryAfterErr);
    }
    Ok(42)
}

fn boxed_ok_static_operation(
    value: u32,
) -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> {
    Box::pin(ready(Ok(value)))
}

async fn long_static_operation() -> Result<u32, &'static str> {
    tokio::time::sleep(Duration::from_mins(1)).await;
    Ok(42)
}

fn boxed_long_static_operation() -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>>
{
    Box::pin(long_static_operation())
}

async fn long_fallback_after_notify(
    started: Arc<tokio::sync::Notify>,
) -> Result<u32, CallError<()>> {
    started.notify_one();
    tokio::time::sleep(Duration::from_mins(1)).await;
    Ok(99)
}

#[tokio::test]
async fn pipeline_timeout_wraps_retry() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_secs(5))
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
                .retry_if(|_: &&str| true),
        )
        .build();

    let result = pipeline
        .call(move || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &str>("fail")
            })
        })
        .await;

    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted { attempts: 3, .. })
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pipeline_warns_on_bad_layer_order() {
    // timeout INSIDE retry is suboptimal — just verify build() succeeds
    let _pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(2)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_millis(1))),
        )
        .timeout(Duration::from_secs(1))
        .build();
}

#[test]
fn build_checked_rejects_out_of_order_steps() {
    let err = ResiliencePipeline::<&str>::builder()
        .retry(RetryConfig::new(2).unwrap())
        .rate_limiter(Arc::new(|| Box::pin(async { Ok(()) })))
        .build_checked()
        .unwrap_err();

    assert_eq!(err.field, "pipeline_order");
}

#[test]
fn build_checked_accepts_recommended_order() {
    let result = ResiliencePipeline::<&str>::builder()
        .load_shed(Arc::new(|| false))
        .rate_limiter(Arc::new(|| Box::pin(async { Ok(()) })))
        .timeout(Duration::from_secs(1))
        .retry(RetryConfig::new(2).unwrap())
        .build_checked();

    assert!(result.is_ok());
}

#[tokio::test]
async fn pipeline_retry_retries_inner_timeout() {
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .timeout(Duration::from_millis(10))
        .build();

    let result = pipeline
        .call(move || {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                let attempt = seen.fetch_add(1, Ordering::SeqCst);
                delay_first_two_attempts(attempt).await;
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pipeline_retry_retries_inner_rate_limit_and_respects_retry_after() {
    let checks = Arc::new(AtomicU32::new(0));
    let seen_checks = Arc::clone(&checks);
    let operations = Arc::new(AtomicU32::new(0));
    let seen_operations = Arc::clone(&operations);
    let retry_after = Duration::from_millis(25);

    let rate_limiter: RateLimitCheck = Arc::new(move || {
        let seen_checks = Arc::clone(&seen_checks);
        Box::pin(async move { reject_first_rate_limit_check(&seen_checks, retry_after) })
    });

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(2)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .rate_limiter(rate_limiter)
        .build();

    let start = std::time::Instant::now();
    let result = pipeline
        .call(move || {
            let seen_operations = Arc::clone(&seen_operations);
            Box::pin(async move {
                seen_operations.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert_eq!(operations.load(Ordering::SeqCst), 1);
    assert!(
        start.elapsed() >= Duration::from_millis(20),
        "retry_after should act as a retry delay floor"
    );
}

#[tokio::test]
async fn pipeline_retry_respects_operation_retry_hint_from_classify_errors() {
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);

    let pipeline = ResiliencePipeline::<RetryAfterErr>::builder()
        .classify_errors()
        .retry(
            RetryConfig::new(2)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .build();

    let start = std::time::Instant::now();
    let result = pipeline
        .call(move || {
            let seen = Arc::clone(&seen);
            Box::pin(async move { fail_once_with_retry_hint(&seen) })
        })
        .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(
        start.elapsed() >= Duration::from_millis(20),
        "Classify retry_hint should act as a retry delay floor"
    );
}

#[test]
fn classify_errors_preserves_user_retry_hint() {
    let builder = ResiliencePipeline::<RetryAfterErr>::builder()
        .retry_hint(|_: &RetryAfterErr| Some(Duration::from_millis(5)))
        .classify_errors();

    let hint = builder
        .retry_hint
        .as_ref()
        .and_then(|hint| hint(&RetryAfterErr));

    assert_eq!(hint, Some(Duration::from_millis(5)));
}

#[tokio::test]
async fn pipeline_retry_does_not_retry_inner_circuit_open() {
    let cb = Arc::new(CircuitBreaker::new(crate::CircuitBreakerConfig::default()).unwrap());
    cb.force_open();
    let operations = Arc::new(AtomicU32::new(0));
    let seen_operations = Arc::clone(&operations);

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .circuit_breaker(cb)
        .build();

    let result = pipeline
        .call(move || {
            let seen_operations = Arc::clone(&seen_operations);
            Box::pin(async move {
                seen_operations.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::CircuitOpen)));
    assert_eq!(operations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn build_recommended_order_rejects_before_retry() {
    let checks = Arc::new(AtomicU32::new(0));
    let seen_checks = Arc::clone(&checks);
    let operations = Arc::new(AtomicU32::new(0));
    let seen_operations = Arc::clone(&operations);

    let rate_limiter: RateLimitCheck = Arc::new(move || {
        let seen_checks = Arc::clone(&seen_checks);
        Box::pin(async move { always_reject_rate_limit_check(&seen_checks) })
    });

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .rate_limiter(rate_limiter)
        .build_recommended_order();

    let result = pipeline
        .call(move || {
            let seen_operations = Arc::clone(&seen_operations);
            Box::pin(async move {
                seen_operations.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::RateLimited { .. })));
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert_eq!(operations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pipeline_with_sink_overrides_retry_config_sink() {
    let sink = RecordingSink::new();

    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .retry(
            RetryConfig::new(2)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO))
                .retry_if(|_: &&str| true),
        )
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
        .await;

    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted { attempts: 2, .. })
    ));
    assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 2);
}

#[tokio::test]
async fn pipeline_returns_ok_on_success() {
    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn pipeline_retry_does_not_replay_unknown_operation_errors_by_default() {
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .build();

    let result = pipeline
        .call(move || {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &str>("unknown")
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::Operation("unknown"))));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pipeline_context_cancels_retry_sleep() {
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = CallContext::from_cancellation(cancellation.clone());

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_secs(10)))
                .retry_if(|_: &&str| true),
        )
        .build();

    let task = tokio::spawn(async move {
        pipeline
            .call_with_context(&context, move || {
                fail_transient_after_count(Arc::clone(&seen))
            })
            .await
    });

    while attempts.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    cancellation.cancel();

    let result = tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .expect("pipeline should stop during retry sleep")
        .expect("task should not panic");

    assert!(matches!(
        result,
        Err(CallError::Cancelled { reason: Some(reason) }) if reason == "shutdown"
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pipeline_retry_preserves_retry_if_predicate() {
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&attempts);

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(RetryConfig::new(3).unwrap().retry_if(|_: &&str| false))
        .build();

    let result = pipeline
        .call(move || {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &str>("permanent")
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::Operation("permanent"))));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pipeline_retry_classifier_blocks_inner_pattern_retries() {
    let checks = Arc::new(AtomicU32::new(0));
    let seen_checks = Arc::clone(&checks);
    let rate_limiter: RateLimitCheck = Arc::new(move || {
        let seen_checks = Arc::clone(&seen_checks);
        Box::pin(
            async move { reject_first_rate_limit_check(&seen_checks, Duration::from_millis(50)) },
        )
    });

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(RetryConfig::new(3).unwrap().retry_if(|_: &&str| false))
        .rate_limiter(rate_limiter)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert!(matches!(result, Err(CallError::RateLimited { .. })));
    assert_eq!(checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pipeline_retry_preserves_retry_hooks() {
    let sink = RecordingSink::new();
    let notifications: Arc<StdMutex<Vec<(u32, Duration)>>> = Arc::new(StdMutex::new(Vec::new()));
    let seen_notifications = Arc::clone(&notifications);
    let attempts = Arc::new(AtomicU32::new(0));
    let seen_attempts = Arc::clone(&attempts);

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
                .with_sink(sink.clone())
                .retry_if(|_: &&str| true)
                .on_retry(move |_err: &&str, delay: Duration, attempt: u32| {
                    seen_notifications.lock().unwrap().push((attempt, delay));
                }),
        )
        .build();

    let result = pipeline
        .call(move || {
            let seen_attempts = Arc::clone(&seen_attempts);
            Box::pin(async move {
                seen_attempts.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &str>("transient")
            })
        })
        .await;

    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted {
            attempts: 3,
            last: "transient",
        })
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
    assert_eq!(notifications.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn pipeline_timeout_fires() {
    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_millis(10))
        .build();

    let result = pipeline
        .call(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
}

#[tokio::test]
async fn pipeline_rate_limiter_inside_cb_does_not_panic() {
    use crate::circuit_breaker::CircuitBreakerConfig;

    let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()).unwrap());

    // Rate limiter that always rejects
    let rl: RateLimitCheck = Arc::new(|| Box::pin(async { Err(CallError::rate_limited()) }));

    let pipeline = ResiliencePipeline::<&str>::builder()
        .circuit_breaker(cb)
        .rate_limiter(rl)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    // Should return RateLimited, not panic
    assert!(matches!(result, Err(CallError::RateLimited { .. })));
}

#[tokio::test]
async fn pipeline_with_sink_emits_timeout_event() {
    let sink = RecordingSink::new();
    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .timeout(Duration::from_millis(10))
        .build();

    let result = pipeline
        .call(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
    assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);
}

#[tokio::test]
async fn pipeline_with_sink_emits_rate_limit_event() {
    let sink = RecordingSink::new();
    let rate_limiter: RateLimitCheck =
        Arc::new(|| Box::pin(async { Err(CallError::rate_limited_after(Duration::from_secs(2))) }));
    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .rate_limiter(rate_limiter)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert!(matches!(
        result,
        Err(CallError::RateLimited {
            retry_after: Some(duration),
        }) if duration == Duration::from_secs(2)
    ));
    assert_eq!(sink.count(ResilienceEventKind::RateLimitExceeded), 1);
}

#[tokio::test]
async fn pipeline_with_sink_emits_load_shed_event() {
    let sink = RecordingSink::new();
    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .load_shed(Arc::new(|| true))
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert!(matches!(result, Err(CallError::LoadShed)));
    assert_eq!(sink.count(ResilienceEventKind::LoadShed), 1);
}

#[tokio::test]
async fn pipeline_with_sink_does_not_double_count_prebuilt_bulkhead_rejection() {
    let sink = RecordingSink::new();
    let bh = Arc::new(
        Bulkhead::new(crate::BulkheadConfig {
            max_concurrency: 1,
            queue_size: 0,
            queue_wait_timeout: None,
        })
        .unwrap()
        .with_sink(sink.clone()),
    );
    let _permit = bh.acquire::<&str>().await.unwrap();

    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .bulkhead(bh)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert!(matches!(result, Err(CallError::BulkheadFull)));
    assert_eq!(sink.count(ResilienceEventKind::BulkheadRejected), 1);
}

#[tokio::test]
async fn pipeline_with_sink_does_not_double_count_prebuilt_circuit_state_change() {
    use crate::CircuitState;

    let sink = RecordingSink::new();
    let cb = Arc::new(
        CircuitBreaker::new(crate::CircuitBreakerConfig {
            failure_threshold: 1,
            min_operations: 1,
            ..Default::default()
        })
        .unwrap()
        .with_sink(sink.clone()),
    );

    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .circuit_breaker(cb)
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
        .await;

    assert!(matches!(result, Err(CallError::Operation("fail"))));
    assert!(sink.has_state_change(CircuitState::Open));
    assert_eq!(sink.count(ResilienceEventKind::CircuitStateChanged), 1);
}

#[tokio::test]
async fn pipeline_cb_half_open_allows_single_probe() {
    use crate::{
        CircuitState,
        circuit_breaker::{CircuitBreakerConfig, Outcome},
    };

    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            reset_timeout: Duration::from_millis(50),
            max_half_open_operations: 1,
            min_operations: 1,
            count_timeouts_as_failures: true,
            ..Default::default()
        })
        .unwrap(),
    );

    // Trip the breaker
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // Wait for reset timeout
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Pipeline should succeed through HalfOpen → Closed
    let pipeline = ResiliencePipeline::<&str>::builder()
        .circuit_breaker(Arc::clone(&cb))
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(cb.circuit_state(), CircuitState::Closed);
}

#[tokio::test]
async fn pipeline_bulkhead_takes_single_permit() {
    let bh = Arc::new(
        Bulkhead::new(crate::BulkheadConfig {
            max_concurrency: 2,
            queue_size: 1,
            queue_wait_timeout: None,
        })
        .unwrap(),
    );

    let pipeline = ResiliencePipeline::<&str>::builder()
        .bulkhead(Arc::clone(&bh))
        .build();

    // After pipeline.call completes, the permit should be released
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;
    assert_eq!(result.unwrap(), 42);

    // Both permits should be available again
    assert_eq!(bh.available_permits(), 2);
}

#[tokio::test]
async fn pipeline_call_with_fallback_recovers() {
    use crate::fallback::ValueFallback;

    let sink = RecordingSink::new();
    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .timeout(Duration::from_millis(10))
        .build();

    let fallback = ValueFallback::new(99u32);

    let result = pipeline
        .call_with_fallback(
            || {
                Box::pin(async {
                    std::future::pending::<()>().await;
                    Ok::<u32, &str>(42)
                })
            },
            &fallback,
        )
        .await;

    assert_eq!(result.unwrap(), 99);
    assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
    assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 1);
    assert_eq!(sink.count(ResilienceEventKind::PipelineCompleted), 1);

    let events = sink.events();
    let completed = events.iter().find_map(|event| {
        if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
            Some((scope, outcome))
        } else {
            None
        }
    });
    assert!(matches!(
        completed,
        Some((
            _,
            PipelineOutcome::FallbackSucceeded {
                primary_error: CallErrorKind::Timeout,
            }
        ))
    ));
}

#[tokio::test]
async fn pipeline_context_fallback_does_not_recover_cancellation() {
    struct AlwaysFallback {
        calls: Arc<AtomicU32>,
    }

    impl crate::fallback::FallbackStrategy<u32, &'static str> for AlwaysFallback {
        fn recover<'a>(
            &'a self,
            _error: CallError<&'static str>,
        ) -> Pin<Box<dyn Future<Output = Result<u32, CallError<&'static str>>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(ready(Ok(99)))
        }

        fn should_fallback(&self, _error: &CallError<&'static str>) -> bool {
            true
        }
    }

    let calls = Arc::new(AtomicU32::new(0));
    let fallback = AlwaysFallback {
        calls: Arc::clone(&calls),
    };
    let cancellation = CancellationContext::with_reason("shutdown");
    cancellation.cancel();
    let context = CallContext::from_cancellation(cancellation);
    let pipeline = ResiliencePipeline::<&'static str>::builder().build();

    let result = pipeline
        .call_with_context_and_fallback(&context, || boxed_ok_static_operation(42), &fallback)
        .await;

    assert!(matches!(result, Err(CallError::Cancelled { .. })));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pipeline_context_cancels_inflight_fallback() {
    use crate::fallback::FunctionFallback;

    let cancellation = CancellationContext::with_reason("shutdown");
    let context = CallContext::from_cancellation(cancellation.clone());
    let started = Arc::new(tokio::sync::Notify::new());
    let started_for_fallback = Arc::clone(&started);

    let task = tokio::spawn(async move {
        let pipeline = ResiliencePipeline::<&'static str>::builder()
            .timeout(Duration::from_millis(1))
            .build();
        let fallback = FunctionFallback::new(move |_err: CallError<()>| {
            let started = Arc::clone(&started_for_fallback);
            long_fallback_after_notify(started)
        });

        pipeline
            .call_with_context_and_fallback(&context, boxed_long_static_operation, &fallback)
            .await
    });

    started.notified().await;
    cancellation.cancel();

    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(result, Err(CallError::Cancelled { .. })));
}

#[tokio::test]
async fn call_context_deadline_bounds_entire_pipeline() {
    let sink = RecordingSink::new();
    let context = CallContext::with_timeout(Duration::from_millis(1))
        .with_scope(EventScope::empty().tenant_id("tenant-context"));
    let pipeline = ResiliencePipeline::<&'static str>::builder()
        .with_sink(sink.clone())
        .scope(EventScope::empty().tenant_id("tenant-builder"))
        .build();

    let result = pipeline
        .call_with_context(&context, boxed_long_static_operation)
        .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
    assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);

    let events = sink.events();
    let completed = events.iter().find_map(|event| {
        if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
            Some((scope, outcome))
        } else {
            None
        }
    });
    assert!(matches!(
        completed,
        Some((
            scope,
            PipelineOutcome::Failure {
                error: CallErrorKind::Timeout,
            }
        )) if scope.tenant_id.as_deref() == Some("tenant-context")
    ));
}

#[tokio::test]
async fn call_context_deadline_bounds_inflight_fallback() {
    use crate::fallback::FunctionFallback;

    let sink = RecordingSink::new();
    let started = Arc::new(tokio::sync::Notify::new());
    let started_for_fallback = Arc::clone(&started);
    let context = CallContext::with_timeout(Duration::from_millis(50));
    let pipeline = ResiliencePipeline::<&'static str>::builder()
        .with_sink(sink.clone())
        .timeout(Duration::from_millis(1))
        .build();
    let fallback = FunctionFallback::new(move |_err: CallError<()>| {
        let started = Arc::clone(&started_for_fallback);
        long_fallback_after_notify(started)
    });

    let call =
        pipeline.call_with_context_and_fallback(&context, boxed_long_static_operation, &fallback);
    tokio::pin!(call);
    tokio::select! {
        () = started.notified() => {},
        result = &mut call => panic!("fallback did not start before call completed: {result:?}"),
    }

    let result = call.await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
    assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
    assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 2);
    assert_eq!(sink.count(ResilienceEventKind::PipelineCompleted), 1);
}

#[tokio::test]
async fn pipeline_completion_event_carries_scope() {
    let sink = RecordingSink::new();
    let pipeline = ResiliencePipeline::<&str>::builder()
        .with_sink(sink.clone())
        .scope(
            EventScope::empty()
                .tenant_id("tenant-a")
                .operation("gmail.poll"),
        )
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;

    assert_eq!(result.unwrap(), 42);
    let events = sink.events();
    let completed = events.iter().find_map(|event| {
        if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
            Some((scope, outcome))
        } else {
            None
        }
    });

    assert!(matches!(
        completed,
        Some((scope, PipelineOutcome::Success))
            if scope.tenant_id.as_deref() == Some("tenant-a")
                && scope.operation.as_deref() == Some("gmail.poll")
    ));
}

#[tokio::test]
async fn pipeline_call_with_fallback_passes_through_on_success() {
    use crate::fallback::ValueFallback;

    let pipeline = ResiliencePipeline::<&str>::builder().build();
    let fallback = ValueFallback::new(0u32);

    let result = pipeline
        .call_with_fallback(|| Box::pin(async { Ok::<u32, &str>(42) }), &fallback)
        .await;

    assert_eq!(result.unwrap(), 42);
}
