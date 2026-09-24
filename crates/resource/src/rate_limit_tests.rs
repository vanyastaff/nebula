use std::{num::NonZeroU32, time::Duration};

use tokio::time::Instant;

use super::*;
use crate::ErrorKind;

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("non-zero test value")
}

fn per_second(requests: u32, burst: u32) -> Rate {
    Rate::per_second(nz(requests))
        .with_burst(nz(burst))
        .unwrap()
}

fn limiter(rate: Rate) -> ResourceLimiter {
    ResourceLimiter::new(
        Arc::new(MemoryLimitStore::new()),
        LimitKey::new("test:row").unwrap(),
        rate,
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    )
}

#[test]
fn without_an_override_the_declared_rate_applies() {
    let declared = per_second(30, 30);
    let policy = ResiliencePolicy::new().rate(declared);
    assert_eq!(policy.effective_rate(None).unwrap(), Some(declared));
    assert_eq!(ResiliencePolicy::new().effective_rate(None).unwrap(), None);
}

#[test]
fn tighten_only_accepts_slower_and_rejects_faster_overrides() {
    let policy = ResiliencePolicy::new().rate(per_second(30, 30));
    assert_eq!(
        policy.effective_rate(Some(per_second(10, 5))).unwrap(),
        Some(per_second(10, 5))
    );
    for looser in [per_second(60, 30), per_second(30, 60)] {
        let error = policy.effective_rate(Some(looser)).unwrap_err();
        assert_eq!(error.kind(), &ErrorKind::Permanent);
    }
    // With nothing declared, any limit is a tightening.
    assert!(
        ResiliencePolicy::new()
            .effective_rate(Some(per_second(5, 1)))
            .is_ok()
    );
}

#[test]
fn fixed_rejects_every_override_and_up_to_caps_at_the_ceiling() {
    let fixed = ResiliencePolicy::new()
        .rate(per_second(30, 30))
        .overrides(Override::Fixed);
    assert!(fixed.effective_rate(Some(per_second(1, 1))).is_err());

    let tiered = ResiliencePolicy::new()
        .rate(per_second(30, 30))
        .overrides(Override::UpTo(per_second(1_000, 1_000)));
    assert!(tiered.effective_rate(Some(per_second(500, 500))).is_ok());
    assert!(
        tiered
            .effective_rate(Some(per_second(2_000, 1_000)))
            .is_err()
    );
}

#[test]
fn settings_parse_strictly_and_publish_a_schema() {
    let parsed: RateLimitSettings = serde_json::from_value(
        serde_json::json!({ "requests": 30, "period_ms": 1000, "burst": 5 }),
    )
    .expect("valid");
    assert_eq!(parsed, RateLimitSettings::new(30, 1_000).with_burst(5));
    assert_eq!(parsed.to_rate().unwrap(), per_second(30, 5));
    for invalid in [
        serde_json::json!({ "requests": 30 }),
        serde_json::json!({ "requests": 30, "period_ms": 1000, "per_second": 1 }),
        serde_json::json!({ "requests": 0, "period_ms": 1000 }),
    ] {
        assert!(
            RateLimitSettings::rate_from_value(Some(&invalid)).is_err(),
            "{invalid}"
        );
    }
    assert_eq!(
        RateLimitSettings::rate_from_value(Some(&serde_json::Value::Null)).unwrap(),
        None
    );
    nebula_schema::schema_of::<RateLimitSettings>().expect("rate limit settings schema");
}

#[tokio::test(start_paused = true)]
async fn ready_waits_within_the_deadline_and_fails_fast_beyond_it() {
    let limiter = limiter(per_second(10, 1));
    limiter.ready(None).await.expect("free");
    let started = Instant::now();
    limiter
        .ready(Some(std::time::Instant::now() + Duration::from_secs(1)))
        .await
        .expect("slot within the deadline");
    assert_eq!(started.elapsed(), Duration::from_millis(100));

    let started = Instant::now();
    let error = limiter
        .ready(Some(std::time::Instant::now()))
        .await
        .expect_err("next slot is 100 ms away, the deadline is now");
    assert_eq!(started.elapsed(), Duration::ZERO, "no pointless wait");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_millis(100)
    ));
}

#[tokio::test(start_paused = true)]
async fn penalty_is_capped_by_the_policy() {
    let limiter = limiter(per_second(10, 1));
    limiter
        .penalize(Duration::from_hours(1))
        .await
        .expect("penalty applies");
    let started = Instant::now();
    limiter
        .ready(None)
        .await
        .expect("waits out the capped penalty");
    assert_eq!(started.elapsed(), Duration::from_mins(1));
}

#[tokio::test(start_paused = true)]
async fn a_provider_refusal_blocks_the_key_for_its_retry_after() {
    let limiter = limiter(per_second(100, 1));
    let error = limiter
        .call(None, || async {
            Err::<(), _>(Error::exhausted("429", Some(Duration::from_secs(30))))
        })
        .await
        .expect_err("the call's own error is returned");
    assert!(matches!(error.kind(), ErrorKind::Exhausted { .. }));

    let started = Instant::now();
    limiter
        .call(None, || async { Ok::<_, Error>(()) })
        .await
        .expect("admitted after the block");
    assert_eq!(started.elapsed(), Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn refusals_without_retry_after_back_off_exponentially_and_reset() {
    let limiter = limiter(per_second(100, 1));
    let refuse = || async { Err::<(), _>(Error::exhausted("429", None)) };
    for expected in [1, 2, 4] {
        let _ = limiter.call(None, refuse).await;
        let started = Instant::now();
        limiter.ready(None).await.expect("admitted");
        assert_eq!(started.elapsed(), Duration::from_secs(expected));
    }
    limiter
        .call(None, || async { Ok::<_, Error>(()) })
        .await
        .expect("success");
    let _ = limiter.call(None, refuse).await;
    let started = Instant::now();
    limiter.ready(None).await.expect("admitted");
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(1),
        "a success resets the backoff"
    );
}

#[test]
fn retry_after_header_parses_delay_seconds_only() {
    assert_eq!(
        retry_after_from_header(" 120 "),
        Some(Duration::from_mins(2))
    );
    assert_eq!(
        retry_after_from_header("Wed, 21 Oct 2015 07:28:00 GMT"),
        None
    );
    assert_eq!(retry_after_from_header("-1"), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_never_exceed_the_burst() {
    // One request per hour: within the test only the burst can pass.
    let rate = Rate::new(nz(1), Duration::from_hours(1))
        .unwrap()
        .with_burst(nz(5))
        .unwrap();
    let limiter = Arc::new(limiter(rate));
    let barrier = Arc::new(tokio::sync::Barrier::new(64));
    let mut handles = Vec::new();
    for _ in 0..64 {
        let limiter = Arc::clone(&limiter);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            limiter.ready(Some(std::time::Instant::now())).await.is_ok()
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
