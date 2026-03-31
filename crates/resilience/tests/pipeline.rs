//! Integration tests for ResiliencePipeline — end-to-end multi-layer scenarios.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resilience::CallError;
use nebula_resilience::bulkhead::{Bulkhead, BulkheadConfig};
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::fallback::ValueFallback;
use nebula_resilience::pipeline::{RateLimitCheck, ResiliencePipeline};
use nebula_resilience::rate_limiter::TokenBucket;
use nebula_resilience::retry::{BackoffConfig, RetryConfig};

// ── Regression: Pipeline total_budget stops retries ─────────────────────────

#[tokio::test]
async fn pipeline_total_budget_limits_retries() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(100)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_millis(30)))
                .total_budget(Duration::from_millis(100)),
        )
        .build();

    let start = std::time::Instant::now();
    let _ = pipeline
        .call(move || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &str>("fail")
            })
        })
        .await;
    let elapsed = start.elapsed();

    let attempts = counter.load(Ordering::SeqCst);
    // With 30ms backoff and 100ms budget, should stop at ~3-4 attempts
    assert!(
        attempts <= 6,
        "expected budget to limit retries, got {attempts} attempts"
    );
    assert!(
        elapsed < Duration::from_millis(300),
        "expected budget to cap time, took {elapsed:?}"
    );
}

// ── Full-stack pipeline: happy path ─────────────────────────────────────────

#[tokio::test]
async fn full_stack_pipeline_happy_path() {
    let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()).unwrap());
    let bh = Arc::new(
        Bulkhead::new(BulkheadConfig {
            max_concurrency: 10,
            ..Default::default()
        })
        .unwrap(),
    );
    let rl = Arc::new(TokenBucket::new(100, 100.0).unwrap());

    let pipeline = ResiliencePipeline::<&str>::builder()
        .rate_limiter_from(rl)
        .timeout(Duration::from_secs(5))
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::from_millis(10))),
        )
        .circuit_breaker(cb.clone())
        .bulkhead(bh.clone())
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Ok::<_, &str>(42u32) }))
        .await;

    assert_eq!(result.unwrap(), 42);
    // Permits released, CB still closed
    assert_eq!(bh.available_permits(), 10);
    assert_eq!(
        cb.circuit_state(),
        nebula_resilience::sink::CircuitState::Closed
    );
}

// ── Full-stack: retry exhaustion → CB stays closed ──────────────────────────

#[tokio::test]
async fn full_stack_retry_exhaustion_does_not_trip_cb() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 10,
            min_operations: 5,
            ..Default::default()
        })
        .unwrap(),
    );

    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_secs(5))
        .retry(
            RetryConfig::new(3)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .circuit_breaker(cb.clone())
        .build();

    let result = pipeline
        .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
        .await;

    assert!(matches!(
        result,
        Err(CallError::RetriesExhausted { attempts: 3, .. })
    ));
    // CB saw 3 failures but threshold is 10 — still closed
    assert_eq!(
        cb.circuit_state(),
        nebula_resilience::sink::CircuitState::Closed
    );
}

// ── Full-stack: CB trips after enough pipeline calls ────────────────────────

#[tokio::test]
async fn full_stack_cb_trips_after_threshold() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            min_operations: 1,
            reset_timeout: Duration::from_secs(60),
            ..Default::default()
        })
        .unwrap(),
    );

    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(
            RetryConfig::new(1)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .circuit_breaker(cb.clone())
        .build();

    // 3 failures → CB trips
    for _ in 0..3 {
        let _ = pipeline
            .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
            .await;
    }

    assert_eq!(
        cb.circuit_state(),
        nebula_resilience::sink::CircuitState::Open
    );

    // Next call rejected immediately by CB
    let result = pipeline
        .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
        .await;
    assert!(matches!(result, Err(CallError::CircuitOpen)));
}

// ── Full-stack: rate limiter rejection before retry ─────────────────────────

#[tokio::test]
async fn full_stack_rate_limiter_rejects_before_retry() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();

    // Rate limiter that always rejects
    let rl: RateLimitCheck =
        Arc::new(|| Box::pin(async { Err(CallError::RateLimited { retry_after: None }) }));

    let pipeline = ResiliencePipeline::<&str>::builder()
        .rate_limiter(rl)
        .retry(
            RetryConfig::new(5)
                .unwrap()
                .backoff(BackoffConfig::Fixed(Duration::ZERO)),
        )
        .build();

    let result = pipeline
        .call(move || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &str>(42)
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::RateLimited { .. })));
    // Operation never called — rate limiter is before retry
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

// ── Full-stack: pipeline + fallback recovery ────────────────────────────────

#[tokio::test]
async fn full_stack_pipeline_with_fallback_recovers() {
    let pipeline = ResiliencePipeline::<&str>::builder()
        .timeout(Duration::from_millis(10))
        .build();

    let fallback = ValueFallback::new(99u32);

    let result = pipeline
        .call_with_fallback(
            || {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<u32, &str>(42)
                })
            },
            &fallback,
        )
        .await;

    assert_eq!(result.unwrap(), 99);
}
