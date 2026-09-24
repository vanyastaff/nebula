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
        Some(Quota::new(
            Arc::new(MemoryLimitStore::new()),
            LimitKey::new("test:row").unwrap(),
            rate,
        )),
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    )
}

/// A provider error in the shape libraries use (`teloxide::RequestError`).
#[derive(Debug)]
enum ProviderError {
    RetryAfter(Duration),
    Throttled,
    Other,
}

fn provider_throttle() -> OnError<impl Fn(&ProviderError) -> Verdict + Send + Sync> {
    on_error(|error: &ProviderError| match error {
        ProviderError::RetryAfter(after) => Verdict::Throttled {
            retry_after: Some(*after),
        },
        ProviderError::Throttled => Verdict::Throttled { retry_after: None },
        ProviderError::Other => Verdict::Pass,
    })
}

/// Time until `limits` admits the next call.
async fn wait_for_slot(limits: &ResourceLimiter) -> Duration {
    let started = Instant::now();
    limits.ready(None).await.expect("admitted");
    started.elapsed()
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
async fn a_throttled_call_pauses_the_quota_for_its_retry_after() {
    let client = Arc::new(limiter(per_second(100, 1))).wrap("client", provider_throttle());
    let error = client
        .run(async |_| Err::<(), _>(ProviderError::RetryAfter(Duration::from_secs(30))))
        .await
        .expect_err("the client's own error comes back");
    assert!(matches!(
        error,
        LimitedError::Call(ProviderError::RetryAfter(_))
    ));
    assert_eq!(
        wait_for_slot(client.limits()).await,
        Duration::from_secs(30)
    );
}

#[tokio::test(start_paused = true)]
async fn unsignalled_throttling_backs_off_exponentially_and_resets() {
    let client = Arc::new(limiter(per_second(100, 1))).wrap((), provider_throttle());
    let throttled = async |(): &()| Err::<(), _>(ProviderError::Throttled);
    for expected in [1, 2, 4] {
        let _ = client.run(throttled).await;
        assert_eq!(
            wait_for_slot(client.limits()).await,
            Duration::from_secs(expected)
        );
    }
    // Any other outcome, success or not, resets the backoff.
    let _ = client
        .run(async |()| Err::<(), _>(ProviderError::Other))
        .await;
    let _ = client.run(throttled).await;
    assert_eq!(wait_for_slot(client.limits()).await, Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_limit_hit_elsewhere_inside_the_call_does_not_pause_this_quota() {
    let client = Arc::new(limiter(per_second(100, 1))).wrap((), NoThrottle);
    // A nested resource answered "exhausted": only this client's throttle
    // decides what is a provider refusal, so this quota stays open.
    let error = client
        .run(async |()| -> Result<(), Error> {
            Err(Error::exhausted("nested", Some(Duration::from_hours(1))))
        })
        .await;
    assert!(matches!(error, Err(LimitedError::Call(_))));
    assert!(wait_for_slot(client.limits()).await < Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_limiter_without_a_rate_only_honours_pauses() {
    let limits = ResourceLimiter::detached();
    assert_eq!(limits.rate(), None);
    for _ in 0..1_000 {
        assert_eq!(wait_for_slot(&limits).await, Duration::ZERO);
    }
    let client = limits.wrap((), provider_throttle());
    let _ = client
        .run(async |()| Err::<(), _>(ProviderError::RetryAfter(Duration::from_secs(20))))
        .await;
    let error = limits
        .ready(Some(std::time::Instant::now()))
        .await
        .expect_err("paused past the deadline");
    assert!(matches!(
        error.kind(),
        ErrorKind::Exhausted { retry_after: Some(after) } if *after == Duration::from_secs(20)
    ));
    assert_eq!(wait_for_slot(&limits).await, Duration::from_secs(20));
    assert_eq!(wait_for_slot(&limits).await, Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn a_refused_permit_never_reaches_the_client() {
    let client = Arc::new(limiter(per_second(10, 1))).wrap((), NoThrottle);
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .unwrap();
    let reached = AtomicBool::new(false);
    let error = client
        .run_until(Some(std::time::Instant::now()), async |()| {
            reached.store(true, Ordering::Relaxed);
            Ok::<_, ProviderError>(())
        })
        .await
        .expect_err("next slot is past the deadline");
    assert!(matches!(error, LimitedError::Limit(_)));
    assert!(!reached.load(Ordering::Relaxed));
}

#[test]
fn retry_after_header_parses_seconds_and_http_dates() {
    assert_eq!(
        retry_after_from_header(" 120 "),
        Some(Duration::from_mins(2))
    );
    assert_eq!(
        retry_after_from_header("Wed, 21 Oct 2015 07:28:00 GMT"),
        Some(Duration::ZERO),
        "a date in the past means now"
    );
    let in_an_hour =
        httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_hours(1));
    let parsed = retry_after_from_header(&in_an_hour).expect("an HTTP date");
    assert!(parsed > Duration::from_mins(59) && parsed <= Duration::from_hours(1));
    assert_eq!(retry_after_from_header("-1"), None);
    assert_eq!(retry_after_from_header("soon"), None);
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
