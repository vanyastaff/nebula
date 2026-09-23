use std::time::Duration;

use tokio::time::Instant;

use super::*;
use crate::ErrorKind;

fn limiter(requests: u32, period_ms: u64, burst: Option<u32>) -> RateLimiter {
    let mut settings = RateLimitSettings::new(requests, period_ms);
    settings.burst = burst;
    RateLimiter::new(settings).expect("valid settings")
}

#[test]
fn invalid_settings_are_rejected() {
    for settings in [
        RateLimitSettings::new(0, 1_000),
        RateLimitSettings::new(10, 0),
        RateLimitSettings::new(10, 1_000).with_burst(0),
        // 1 ms / 2 million requests is below one nanosecond per request.
        RateLimitSettings::new(2_000_000, 1),
    ] {
        assert!(RateLimiter::new(settings).is_err(), "{settings:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn burst_passes_immediately_then_the_steady_rate_applies() {
    // 10 per second = one per 100 ms, with a burst of 3.
    let limiter = limiter(10, 1_000, Some(3));
    for _ in 0..3 {
        assert_eq!(limiter.reserve(1, None), Ok(Duration::ZERO));
    }
    assert_eq!(limiter.reserve(1, None), Ok(Duration::from_millis(100)));
    assert_eq!(limiter.reserve(1, None), Ok(Duration::from_millis(200)));
}

#[tokio::test(start_paused = true)]
async fn idle_time_restores_the_burst() {
    let limiter = limiter(10, 1_000, Some(2));
    limiter.reserve(2, None).expect("burst");
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(limiter.reserve(2, None), Ok(Duration::ZERO));
}

#[tokio::test(start_paused = true)]
async fn a_slot_past_the_deadline_is_denied_without_consuming_it() {
    let limiter = limiter(1, 1_000, None);
    limiter.reserve(1, None).expect("first request is free");
    assert_eq!(
        limiter.reserve(1, Some(Duration::from_millis(500))),
        Err(RateLimitDenial::Later(Duration::from_secs(1)))
    );
    // The denial consumed nothing: the next slot is still one period away.
    assert_eq!(limiter.reserve(1, None), Ok(Duration::from_secs(1)));
}

#[tokio::test(start_paused = true)]
async fn until_ready_waits_within_the_deadline() {
    let limiter = limiter(10, 1_000, None);
    limiter.until_ready(1, None).await.expect("free");
    let started = Instant::now();
    limiter
        .until_ready(1, Some(Instant::now() + Duration::from_secs(1)))
        .await
        .expect("slot within the deadline");
    assert_eq!(started.elapsed(), Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn until_ready_fails_fast_with_retry_after_when_the_deadline_is_too_close() {
    let limiter = limiter(1, 1_000, None);
    limiter.until_ready(1, None).await.expect("free");
    let started = Instant::now();
    let error = limiter
        .until_ready(1, Some(Instant::now() + Duration::from_millis(10)))
        .await
        .expect_err("slot is after the deadline");
    assert_eq!(started.elapsed(), Duration::ZERO, "no pointless wait");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_secs(1)
    ));
}

#[tokio::test(start_paused = true)]
async fn more_permits_than_the_burst_is_a_permanent_error() {
    let limiter = limiter(10, 1_000, Some(2));
    assert_eq!(limiter.reserve(3, None), Err(RateLimitDenial::ExceedsBurst));
    let error = limiter
        .until_ready(3, None)
        .await
        .expect_err("never grantable");
    assert_eq!(error.kind(), &ErrorKind::Permanent);
}

#[test]
fn settings_parse_strictly_and_publish_a_schema() {
    let parsed: RateLimitSettings = serde_json::from_value(
        serde_json::json!({ "requests": 30, "period_ms": 1000, "burst": 5 }),
    )
    .expect("valid");
    assert_eq!(parsed, RateLimitSettings::new(30, 1_000).with_burst(5));
    assert!(
        serde_json::from_value::<RateLimitSettings>(serde_json::json!({ "requests": 30 })).is_err()
    );
    assert!(
        serde_json::from_value::<RateLimitSettings>(
            serde_json::json!({ "requests": 30, "period_ms": 1000, "per_second": 1 })
        )
        .is_err()
    );
    nebula_schema::schema_of::<RateLimitSettings>().expect("rate limit settings schema");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_never_exceed_the_burst() {
    // One request per hour: within the test only the burst can pass.
    let limiter = std::sync::Arc::new(limiter(1, 3_600_000, Some(5)));
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(64));
    let mut handles = Vec::new();
    for _ in 0..64 {
        let limiter = std::sync::Arc::clone(&limiter);
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            limiter.reserve(1, Some(Duration::ZERO)).is_ok()
        }));
    }
    let mut admitted = 0;
    for handle in handles {
        if handle.await.expect("task") {
            admitted += 1;
        }
    }
    assert_eq!(
        admitted, 5,
        "exactly the burst is admitted under contention"
    );
}
