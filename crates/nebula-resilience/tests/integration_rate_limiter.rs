//! Integration tests for rate limiter patterns

use nebula_resilience::patterns::rate_limiter::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_token_bucket_rate_limiting() {
    let limiter = TokenBucket::new(10, 10.0); // 10 capacity, 10 req/s

    // Should allow burst
    for _ in 0..10 {
        assert!(limiter.acquire().await.is_ok());
    }

    // Should be rate limited now
    assert!(limiter.acquire().await.is_err());

    // Wait for refill
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(limiter.acquire().await.is_ok());
}

#[tokio::test]
async fn test_sliding_window_accuracy() {
    let limiter = SlidingWindow::new(Duration::from_millis(100), 5);

    // Fill the window
    for _ in 0..5 {
        assert!(limiter.acquire().await.is_ok());
    }

    // Should be limited
    assert!(limiter.acquire().await.is_err());

    // Wait for window to slide
    tokio::time::sleep(Duration::from_millis(110)).await;

    // Should allow more requests
    assert!(limiter.acquire().await.is_ok());
}

#[tokio::test]
async fn test_adaptive_rate_limiter_adjusts() {
    let limiter = AdaptiveRateLimiter::new(100.0, 10.0, 1000.0);

    // Record successes - rate should increase
    for _ in 0..20 {
        limiter.record_success().await;
    }

    let rate_after_success = limiter.current_rate().await;

    // Record errors - rate should decrease
    for _ in 0..10 {
        limiter.record_error().await;
    }

    let rate_after_error = limiter.current_rate().await;

    assert!(
        rate_after_error < rate_after_success,
        "Rate should decrease after errors: {} vs {}",
        rate_after_error,
        rate_after_success
    );
}

#[tokio::test]
async fn test_any_rate_limiter_enum() {
    let token_bucket = AnyRateLimiter::TokenBucket(Arc::new(TokenBucket::new(100, 10.0)));
    let leaky = AnyRateLimiter::LeakyBucket(Arc::new(LeakyBucket::new(100, 10.0)));

    // Both should work through the enum
    assert!(token_bucket.acquire().await.is_ok());
    assert!(leaky.acquire().await.is_ok());
}

#[tokio::test]
async fn test_concurrent_rate_limiting() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let limiter = Arc::new(TokenBucket::new(10, 50.0)); // 10 capacity, 50 req/s
    let success_count = Arc::new(AtomicUsize::new(0));
    let reject_count = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];

    // Spawn 100 concurrent requests
    for _ in 0..100 {
        let limiter = Arc::clone(&limiter);
        let success = Arc::clone(&success_count);
        let reject = Arc::clone(&reject_count);

        handles.push(tokio::spawn(async move {
            if limiter.acquire().await.is_ok() {
                success.fetch_add(1, Ordering::Relaxed);
            } else {
                reject.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    futures::future::join_all(handles).await;

    let successes = success_count.load(Ordering::Relaxed);
    let rejects = reject_count.load(Ordering::Relaxed);

    // Should have limited some requests
    assert!(
        successes <= 15,
        "Expected burst limit, got {} successes",
        successes
    );
    assert!(rejects >= 85, "Expected rejections, got {} rejects", rejects);
    assert_eq!(successes + rejects, 100);
}
