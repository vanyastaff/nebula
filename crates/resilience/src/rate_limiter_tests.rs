use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::PolicyContext;

#[tokio::test]
async fn token_bucket_respects_capacity() {
    let limiter = TokenBucket::new(1, 0.001).unwrap();
    assert!(limiter.acquire().await.is_ok());
    assert!(limiter.acquire().await.is_err());
}

#[tokio::test]
async fn token_bucket_returns_retry_after_hint() {
    let limiter = TokenBucket::new(1, 10.0).unwrap();
    limiter.acquire().await.unwrap();

    let err = limiter.acquire().await.unwrap_err();

    assert!(
        matches!(err, CallError::RateLimited { retry_after: Some(delay) } if delay > Duration::ZERO && delay <= Duration::from_millis(100))
    );
}

#[tokio::test]
async fn rate_limiter_call_preserves_retry_after_hint() {
    let limiter = TokenBucket::new(1, 10.0).unwrap();
    limiter.acquire().await.unwrap();

    let err: CallError<()> = limiter
        .call(|| async { Ok::<(), ()>(()) })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CallError::RateLimited {
            retry_after: Some(_),
        }
    ));
}

#[tokio::test]
async fn policy_context_deadline_bounds_rate_limited_operation() {
    let limiter = TokenBucket::new(1, 0.001).unwrap();
    let context = PolicyContext::with_timeout(Duration::from_millis(1));

    let err = limiter
        .call_with_policy_context(&context, || async {
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok::<(), ()>(())
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CallError::Timeout(_)));
}

#[tokio::test]
async fn erased_rate_limiter_context_acquire_observes_cancellation() {
    let limiter: Arc<dyn ErasedRateLimiter> = Arc::new(TokenBucket::new(1, 0.001).unwrap());
    let cancellation = crate::CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let err = limiter
        .acquire_with_policy_context_boxed(&context)
        .await
        .unwrap_err();

    assert!(matches!(err, CallError::Cancelled { .. }));
}

#[tokio::test]
async fn erased_rate_limiter_forwards_specialized_context_acquire() {
    struct ContextAwareLimiter {
        plain_acquires: AtomicUsize,
        context_acquires: AtomicUsize,
    }

    impl RateLimiter for ContextAwareLimiter {
        async fn acquire(&self) -> Result<(), CallError<()>> {
            self.plain_acquires.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn acquire_with_policy_context<'a>(
            &'a self,
            _context: &'a PolicyContext,
        ) -> Result<(), CallError<()>> {
            self.context_acquires.fetch_add(1, Ordering::SeqCst);
            Err(CallError::cancelled_with("specialized path"))
        }

        async fn current_rate(&self) -> f64 {
            1.0
        }

        async fn reset(&self) {}
    }

    let limiter = Arc::new(ContextAwareLimiter {
        plain_acquires: AtomicUsize::new(0),
        context_acquires: AtomicUsize::new(0),
    });
    let erased: Arc<dyn ErasedRateLimiter> = limiter.clone();

    let err = erased
        .acquire_with_policy_context_boxed(&PolicyContext::empty())
        .await
        .unwrap_err();

    assert!(matches!(err, CallError::Cancelled { .. }));
    assert_eq!(limiter.plain_acquires.load(Ordering::SeqCst), 0);
    assert_eq!(limiter.context_acquires.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn adaptive_rate_limiter_context_call_records_operation_outcome() {
    let limiter = AdaptiveRateLimiter::new(10.0, 1.0, 100.0).unwrap();
    let context = PolicyContext::empty();

    let ok = limiter
        .call_with_policy_context(&context, || async { Ok::<_, ()>(()) })
        .await;
    assert!(ok.is_ok());

    let err = limiter
        .call_with_policy_context(&context, || async { Err::<(), _>(()) })
        .await;
    assert!(matches!(err, Err(CallError::Operation(()))));

    assert_eq!(limiter.success_count.load(Ordering::Relaxed), 1);
    assert_eq!(limiter.error_count.load(Ordering::Relaxed), 1);
}

#[test]
fn token_bucket_update_rate_sanitizes_non_finite_values() {
    let limiter = TokenBucket::new(1, 10.0).unwrap();
    limiter.update_rate(f64::NAN);

    let rate = f64::from_bits(limiter.refill_rate.load(Ordering::Acquire));
    assert!((rate - 0.001).abs() < f64::EPSILON);
}

#[tokio::test]
async fn erased_rate_limiter_registry_stores_heterogeneous_limiters() {
    let registry: Vec<Arc<dyn ErasedRateLimiter>> = vec![
        Arc::new(TokenBucket::new(1, 1.0).unwrap()),
        Arc::new(SlidingWindow::new(Duration::from_secs(1), 1).unwrap()),
    ];

    assert!(registry[0].acquire_boxed().await.is_ok());
    assert!(registry[1].acquire_boxed().await.is_ok());
    assert!(registry[0].current_rate_boxed().await.is_finite());

    registry[0].reset_boxed().await;
    assert!(registry[0].acquire_boxed().await.is_ok());
}

#[test]
fn leaky_bucket_rejects_zero_capacity() {
    assert!(LeakyBucket::new(0, 1.0).is_err());
}

#[test]
fn leaky_bucket_rejects_invalid_leak_rate() {
    assert!(LeakyBucket::new(10, 0.0).is_err());
    assert!(LeakyBucket::new(10, -1.0).is_err());
}

#[test]
fn leaky_bucket_accepts_valid_config() {
    assert!(LeakyBucket::new(10, 1.0).is_ok());
}

#[tokio::test]
async fn leaky_bucket_preserves_fractional_leak_time_between_rejections() {
    let limiter = LeakyBucket::new(1, 4.0).unwrap();

    limiter.acquire().await.unwrap();

    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(matches!(
        limiter.acquire().await,
        Err(CallError::RateLimited { .. })
    ));

    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(matches!(
        limiter.acquire().await,
        Err(CallError::RateLimited { .. })
    ));

    tokio::time::sleep(Duration::from_millis(140)).await;
    assert!(limiter.acquire().await.is_ok());
}

#[tokio::test]
async fn leaky_bucket_returns_retry_after_hint() {
    let limiter = LeakyBucket::new(1, 4.0).unwrap();
    limiter.acquire().await.unwrap();

    let err = limiter.acquire().await.unwrap_err();

    assert!(
        matches!(err, CallError::RateLimited { retry_after: Some(delay) } if delay > Duration::ZERO && delay <= Duration::from_millis(250))
    );
}

#[test]
fn leaky_bucket_partial_drain_preserves_fractional_leak_time() {
    let limiter = LeakyBucket::new(4, 4.0).unwrap();
    let start = Instant::now();
    let now = start + Duration::from_millis(600);

    let (level, leaked_duration) = {
        let mut state = limiter.state.lock();
        state.level = 3;
        state.last_leak = start;

        LeakyBucket::leak_locked(&mut state, limiter.leak_rate, now);

        (state.level, state.last_leak.duration_since(start))
    };

    assert_eq!(level, 1);
    assert_eq!(leaked_duration, Duration::from_millis(500));
}

#[test]
fn sliding_window_rejects_zero_requests() {
    assert!(SlidingWindow::new(Duration::from_secs(1), 0).is_err());
}

#[test]
fn sliding_window_rejects_zero_duration() {
    assert!(SlidingWindow::new(Duration::ZERO, 10).is_err());
}

#[test]
fn sliding_window_accepts_valid_config() {
    assert!(SlidingWindow::new(Duration::from_secs(1), 10).is_ok());
}

#[tokio::test]
async fn sliding_window_returns_retry_after_hint() {
    let limiter = SlidingWindow::new(Duration::from_millis(100), 1).unwrap();
    limiter.acquire().await.unwrap();

    let err = limiter.acquire().await.unwrap_err();

    assert!(
        matches!(err, CallError::RateLimited { retry_after: Some(delay) } if delay > Duration::ZERO && delay <= Duration::from_millis(100))
    );
}

// ── B2: AdaptiveRateLimiter rejects initial_rate outside bounds ──────

#[test]
fn adaptive_rejects_initial_rate_below_min() {
    let result = AdaptiveRateLimiter::new(1.0, 10.0, 100.0);
    assert!(result.is_err(), "should reject initial_rate below min_rate");
}

#[test]
fn adaptive_rejects_initial_rate_above_max() {
    let result = AdaptiveRateLimiter::new(500.0, 10.0, 100.0);
    assert!(result.is_err(), "should reject initial_rate above max_rate");
}

#[test]
fn adaptive_accepts_initial_rate_at_bounds() {
    assert!(AdaptiveRateLimiter::new(10.0, 10.0, 100.0).is_ok());
    assert!(AdaptiveRateLimiter::new(100.0, 10.0, 100.0).is_ok());
}

// ── B1: update_burst keeps burst in sync with rate ──────────────────

#[tokio::test]
async fn token_bucket_update_burst_limits_tokens() {
    let limiter = TokenBucket::new(10, 0.001).unwrap();
    // Exhaust initial tokens
    for _ in 0..10 {
        assert!(limiter.acquire().await.is_ok());
    }
    assert!(limiter.acquire().await.is_err());

    // Reset and reduce burst to 3
    limiter.reset().await;
    limiter.update_burst(3);

    // Should only get 3 tokens now (burst caps refill)
    for _ in 0..3 {
        assert!(limiter.acquire().await.is_ok());
    }
    assert!(limiter.acquire().await.is_err());
}

// ── M3: atomic counters work correctly ──────────────────────────────

#[tokio::test]
async fn adaptive_record_success_and_error_are_lock_free() {
    let limiter = AdaptiveRateLimiter::new(50.0, 10.0, 100.0).unwrap();
    // Should not deadlock or panic with many concurrent calls
    for _ in 0..100 {
        limiter.record_success();
    }
    for _ in 0..50 {
        limiter.record_error();
    }
    // Rate should still be around initial since stats_window (1 min) hasn't elapsed
    let rate = limiter.current_rate().await;
    assert!((rate - 50.0).abs() < 0.001, "expected ~50.0, got {rate}");
}
