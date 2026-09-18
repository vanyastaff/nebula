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
async fn status_and_acquire_agree_after_expiry() {
    let limiter = SlidingWindow::new(Duration::from_millis(50), 10).unwrap();

    // Add 5 requests: the window now has 5 of its 10-permit quota used.
    for _ in 0..5 {
        limiter.acquire().await.unwrap();
    }
    let before_expiry = limiter.status().await;
    assert_eq!(
        before_expiry.remaining, 5.0,
        "a half-used window must report its remaining quota"
    );
    assert_eq!(
        before_expiry.limit_per_second, None,
        "a window counter enforces a count per window, not a rate"
    );

    // Wait for expiry: full quota returns, and acquire() agrees.
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        limiter.status().await.remaining,
        10.0,
        "expired entries must not count against the remaining quota"
    );

    for _ in 0..10 {
        assert!(limiter.acquire().await.is_ok());
    }
    assert!(limiter.acquire().await.is_err());
}
