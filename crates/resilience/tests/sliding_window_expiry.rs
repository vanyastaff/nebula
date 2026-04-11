//! Regression test: SlidingWindow must evict expired entries on every acquire(),
//! not only when the deque reaches max_requests.
//!
//! Before the fix, a window with 9/10 stale entries would only allow 1 new
//! request until the deque filled up and triggered cleanup.

use std::time::Duration;

use nebula_resilience::rate_limiter::{RateLimiter, SlidingWindow};

#[tokio::test]
async fn expired_entries_do_not_block_new_requests() {
    // Window: 50ms, max 5 requests
    let limiter = SlidingWindow::new(Duration::from_millis(50), 5).unwrap();

    // Fill 3 of 5 slots (below max_requests)
    for _ in 0..3 {
        assert!(limiter.acquire().await.is_ok());
    }

    // Wait for all 3 to expire
    tokio::time::sleep(Duration::from_millis(60)).await;

    // All 3 entries are now outside the window.
    // We should be able to make 5 new requests (full capacity),
    // not just 2 (5 - 3 stale entries).
    for i in 0..5 {
        assert!(
            limiter.acquire().await.is_ok(),
            "request {i} should succeed after window expiry"
        );
    }

    // 6th should be rejected (window is full with 5 fresh entries)
    assert!(limiter.acquire().await.is_err());
}

#[tokio::test]
async fn current_rate_and_acquire_agree_after_expiry() {
    let limiter = SlidingWindow::new(Duration::from_millis(50), 10).unwrap();

    // Add 5 requests
    for _ in 0..5 {
        limiter.acquire().await.unwrap();
    }

    // Wait for expiry
    tokio::time::sleep(Duration::from_millis(60)).await;

    // current_rate() should report 0 active requests
    let rate = limiter.current_rate().await;
    assert!(
        rate < 1.0,
        "expected ~0 active requests after expiry, got {rate}"
    );

    // acquire() should also see 0 active and allow full capacity
    for _ in 0..10 {
        assert!(limiter.acquire().await.is_ok());
    }
    assert!(limiter.acquire().await.is_err());
}
