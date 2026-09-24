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
        None,
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    )
}

/// An account limit of `account` plus one message per second per chat.
fn chat_limiter(account: Rate) -> (Arc<ResourceLimiter>, Arc<MemoryLimitStore>) {
    let store = Arc::new(MemoryLimitStore::new());
    let base = LimitKey::new("acct:test").unwrap();
    let shared: Arc<dyn ErasedLimitStore> = store.clone();
    let limiter = ResourceLimiter::new(
        Some(Quota::new(Arc::clone(&shared), base.clone(), account)),
        Some(KeyedLimits::new(
            shared,
            base,
            vec![("chat_id", per_second(1, 1))],
        )),
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    (Arc::new(limiter), store)
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
        serde_json::json!({ "rate": { "requests": 30 } }),
        serde_json::json!({ "rate": { "requests": 30, "period_ms": 1000, "per_second": 1 } }),
        serde_json::json!({ "rate": { "requests": 0, "period_ms": 1000 } }),
        serde_json::json!({ "requests": 30, "period_ms": 1000 }),
    ] {
        assert!(
            ResilienceOverride::from_value(Some(&invalid))
                .and_then(|document| document.requested_rate())
                .is_err(),
            "{invalid}"
        );
    }
    assert_eq!(
        ResilienceOverride::from_value(Some(&serde_json::Value::Null)).unwrap(),
        ResilienceOverride::default()
    );
    nebula_schema::schema_of::<ResilienceOverride>().expect("resilience override schema");
}

#[test]
fn override_refusals_name_the_field_and_rule_never_the_values() {
    let declared = ResiliencePolicy::new().rate(per_second(30, 30));
    let faster = ResilienceOverride::rate(RateLimitSettings::new(9_999, 1_000));
    let error = faster.apply(&declared).expect_err("tighten only");
    let message = error.to_string();
    assert!(
        message.starts_with("resilience_override.rate:"),
        "{message}"
    );
    assert!(!message.contains("9999"), "{message}");

    let fixed = declared.clone().overrides(Override::Fixed);
    let slower = ResilienceOverride::rate(RateLimitSettings::new(1, 1_000));
    assert!(slower.apply(&fixed).is_err());
    assert_eq!(
        slower.apply(&declared).unwrap(),
        Some(per_second(1, 1)),
        "a slower rate is a tightening"
    );
    assert_eq!(
        ResilienceOverride::default().apply(&declared).unwrap(),
        Some(per_second(30, 30)),
        "no override enforces the declared rate"
    );
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
    // Each step is jittered over its upper half: 0.5–1 s, 1–2 s, 2–4 s.
    // Waits include one 10 ms emission interval of the 100/s rate.
    let within = |wait: Duration, step: u64| {
        let step = Duration::from_secs(step);
        wait >= step / 2 && wait <= step + Duration::from_millis(10)
    };
    for step in [1, 2, 4] {
        let _ = client.run(throttled).await;
        let wait = wait_for_slot(client.limits()).await;
        assert!(within(wait, step), "step {step} s waited {wait:?}");
    }
    // Any other outcome, success or not, resets the backoff.
    let _ = client
        .run(async |()| Err::<(), _>(ProviderError::Other))
        .await;
    let _ = client.run(throttled).await;
    let wait = wait_for_slot(client.limits()).await;
    assert!(within(wait, 1), "after a reset waited {wait:?}");
}

#[tokio::test(start_paused = true)]
async fn one_keys_refusals_do_not_escalate_another_keys_backoff() {
    let (limits, _) = chat_limiter(per_second(100, 100));
    let client = limits.wrap((), |outcome: &Result<(), ProviderError>| match outcome {
        Err(ProviderError::Throttled) => Verdict::KeyThrottled { retry_after: None },
        _ => Verdict::Pass,
    });
    let refused = async |(): &()| Err::<(), _>(ProviderError::Throttled);
    // Chat 1 is refused three times; its pauses escalate.
    for _ in 0..3 {
        let _ = client.run_for("chat_id", 1, refused).await;
    }
    // Chat 2's first refusal starts from the first step, 0.5–1 s.
    let _ = client.run_for("chat_id", 2, refused).await;
    let started = Instant::now();
    client
        .run_for("chat_id", 2, async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("admitted after its own pause");
    assert!(
        started.elapsed() <= Duration::from_secs(1) + Duration::from_millis(10),
        "chat 2 waited {:?}, escalated by chat 1",
        started.elapsed()
    );
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

#[tokio::test(start_paused = true)]
async fn each_chat_has_its_own_limit_under_the_account_limit() {
    let (limits, _) = chat_limiter(per_second(100, 100));
    limits.ready_for("chat_id", 42, None).await.expect("free");
    let started = Instant::now();
    limits
        .ready_for("chat_id", 7, None)
        .await
        .expect("another chat");
    assert_eq!(started.elapsed(), Duration::ZERO, "chats do not share");
    limits
        .ready_for("chat_id", 42, None)
        .await
        .expect("waits for its chat");
    assert_eq!(started.elapsed(), Duration::from_secs(1));

    let error = limits
        .ready_for("chat_id", 42, Some(std::time::Instant::now()))
        .await
        .expect_err("the chat's next slot is a second away");
    assert!(matches!(error.kind(), ErrorKind::Exhausted { .. }));
}

#[tokio::test(start_paused = true)]
async fn the_account_limit_still_binds_keyed_calls() {
    // The account allows one call per second; chats would allow more.
    let (limits, _) = chat_limiter(per_second(1, 1));
    limits.ready_for("chat_id", 1, None).await.expect("free");
    let error = limits
        .ready_for("chat_id", 2, Some(std::time::Instant::now()))
        .await
        .expect_err("a fresh chat, but the account is spent");
    assert!(matches!(error.kind(), ErrorKind::Exhausted { .. }));
    // The refused call gave its chat slot back: chat 2 is still free once
    // the account allows.
    let started = Instant::now();
    limits
        .ready_for("chat_id", 2, None)
        .await
        .expect("admitted");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_key_throttle_pauses_only_that_key() {
    let (limits, _) = chat_limiter(per_second(100, 100));
    let client = limits.wrap((), |outcome: &Result<(), ProviderError>| match outcome {
        Err(ProviderError::RetryAfter(after)) => Verdict::KeyThrottled {
            retry_after: Some(*after),
        },
        _ => Verdict::Pass,
    });
    let _ = client
        .run_for("chat_id", 42, async |()| {
            Err::<(), _>(ProviderError::RetryAfter(Duration::from_secs(20)))
        })
        .await;
    let started = Instant::now();
    client
        .run_for("chat_id", 7, async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("other chats and the account are not paused");
    assert_eq!(started.elapsed(), Duration::ZERO);
    client
        .run_for("chat_id", 42, async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("the paused chat waits it out");
    assert_eq!(started.elapsed(), Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn an_undeclared_dimension_is_a_programming_error() {
    let (limits, _) = chat_limiter(per_second(100, 100));
    let error = limits
        .ready_for("user_id", 1, None)
        .await
        .expect_err("not declared");
    assert_eq!(error.kind(), &ErrorKind::Permanent);
    let error = limiter(per_second(1, 1))
        .ready_for("chat_id", 1, None)
        .await
        .expect_err("no per-key limits at all");
    assert_eq!(error.kind(), &ErrorKind::Permanent);
}

#[test]
fn key_values_are_hashed_before_they_reach_a_store() {
    let keyed = KeyedLimits::new(
        Arc::new(MemoryLimitStore::new()),
        LimitKey::new("acct:tenant").unwrap(),
        vec![("email", per_second(1, 1))],
    );
    let (key, _) = keyed
        .limit_for("email", "alice@example.com")
        .expect("valid");
    assert!(key.as_str().starts_with("acct:tenant:k:email:"));
    assert!(!key.as_str().contains("alice"), "{key:?}");
    let (again, _) = keyed.limit_for("email", "alice@example.com").unwrap();
    let (other, _) = keyed.limit_for("email", "bob@example.com").unwrap();
    assert_eq!(key, again, "stable across calls and processes");
    assert_ne!(key, other);
}

#[test]
fn keyed_overrides_follow_the_policy_per_dimension() {
    let policy = ResiliencePolicy::new()
        .rate(per_second(30, 30))
        .keyed("chat_id", per_second(1, 1));
    assert_eq!(
        policy.effective_keyed(&[]).unwrap(),
        vec![("chat_id", per_second(1, 1))]
    );
    let slower = Rate::new(nz(1), Duration::from_secs(2)).unwrap();
    assert_eq!(
        policy
            .effective_keyed(&[("chat_id".to_owned(), slower)])
            .unwrap(),
        vec![("chat_id", slower)]
    );
    let error = policy
        .effective_keyed(&[("chat_id".to_owned(), per_second(5, 1))])
        .expect_err("tighten only");
    assert!(error.to_string().contains("resilience_override.keyed:"));
    let error = policy
        .effective_keyed(&[("secret-dimension".to_owned(), slower)])
        .expect_err("undeclared");
    assert!(!error.to_string().contains("secret-dimension"), "{error}");

    // `UpTo` lets a dimension rise to its own declared rate, no further.
    let tiered = policy.overrides(Override::UpTo(per_second(1_000, 1_000)));
    assert!(
        tiered
            .effective_keyed(&[("chat_id".to_owned(), per_second(2, 1))])
            .is_err()
    );
}

#[test]
fn override_documents_carry_keyed_limits() {
    let document = ResilienceOverride::from_value(Some(&serde_json::json!({
        "keyed": [{ "dimension": "chat_id", "rate": { "requests": 1, "period_ms": 2000 } }]
    })))
    .expect("valid");
    let policy = ResiliencePolicy::new().keyed("chat_id", per_second(1, 1));
    assert_eq!(
        document.apply_keyed(&policy).unwrap(),
        vec![("chat_id", Rate::new(nz(1), Duration::from_secs(2)).unwrap())]
    );
    let twice = ResilienceOverride::default()
        .with_keyed("chat_id", RateLimitSettings::new(1, 2_000))
        .with_keyed("chat_id", RateLimitSettings::new(1, 3_000));
    assert!(twice.requested_keyed().is_err(), "a dimension named twice");
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
