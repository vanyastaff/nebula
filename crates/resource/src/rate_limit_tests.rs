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
            // A second of deadline: the slot after the burst is an hour
            // away, and a deadline already passed would refuse even the
            // burst.
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            limiter.ready(Some(deadline)).await.is_ok()
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

/// A penalty cap as large as `Duration::MAX` pauses without overflowing
/// the pause deadline, for the account and for a key.
#[tokio::test(start_paused = true)]
async fn an_unbounded_penalty_cap_saturates() {
    let store: Arc<dyn ErasedLimitStore> = Arc::new(MemoryLimitStore::new());
    let base = LimitKey::new("acct:test").unwrap();
    let limits = ResourceLimiter::new(
        Some(Quota::new(
            Arc::clone(&store),
            base.clone(),
            per_second(100, 100),
        )),
        Some(KeyedLimits::new(
            store,
            base,
            vec![("chat_id", per_second(1, 1))],
        )),
        Duration::MAX,
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    limits
        .penalize_for("chat_id", 1, Duration::MAX)
        .await
        .expect("a key pause saturates");
    limits
        .penalize(Duration::MAX)
        .await
        .expect("an account pause saturates");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let error = limits
        .ready(Some(deadline))
        .await
        .expect_err("paused far past the deadline");
    assert!(
        matches!(error.kind(), ErrorKind::Exhausted { .. }),
        "{error}"
    );
}

/// A wake-up scheduled within the deadline but run after it (a busy
/// executor) does not admit the call.
#[tokio::test(flavor = "current_thread")]
async fn a_late_wake_up_past_the_deadline_is_refused() {
    let limits = limiter(Rate::new(nz(1), Duration::from_millis(20)).unwrap());
    limits.ready(None).await.expect("the burst passes");
    let deadline = std::time::Instant::now() + Duration::from_millis(40);
    let waiter = tokio::spawn(async move { limits.ready(Some(deadline)).await });
    // Let the waiter book its slot and go to sleep, then hold the only
    // executor thread past the deadline.
    tokio::task::yield_now().await;
    std::thread::sleep(Duration::from_millis(80));
    let error = waiter
        .await
        .expect("waiter task")
        .expect_err("woke after the deadline");
    assert!(
        matches!(error.kind(), ErrorKind::Exhausted { .. }),
        "{error}"
    );
}

/// Behind an account backlog, calls for one key still keep that key's
/// spacing: each runs at an instant both limits allow, not at the later
/// of two slots booked apart.
#[tokio::test(start_paused = true)]
async fn an_account_backlog_does_not_bunch_one_keys_calls() {
    let account = Rate::new(nz(10), Duration::from_secs(1))
        .unwrap()
        .with_burst(nz(1))
        .unwrap();
    let (limits, store) = chat_limiter(account);
    nebula_resilience::rate_limiter::gcra::LimitStore::penalize(
        &*store,
        &LimitKey::new("acct:test").unwrap(),
        &account,
        Duration::from_secs(10),
        Duration::from_mins(1),
    )
    .await
    .unwrap();

    let started = Instant::now();
    let call = || {
        let limits = Arc::clone(&limits);
        async move {
            limits
                .ready_for("chat_id", 42, None)
                .await
                .expect("admitted");
            started.elapsed()
        }
    };
    let (first, second) = tokio::join!(call(), call());
    let (first, second) = (first.min(second), first.max(second));
    assert!(first >= Duration::from_secs(10), "{first:?}");
    assert!(
        second.saturating_sub(first) >= Duration::from_secs(1),
        "the chat's calls stay a second apart: {first:?}, {second:?}"
    );
}

/// A key's pause reaches a caller of this limiter already sleeping on that
/// key, however many other keys are paused.
#[tokio::test(start_paused = true)]
async fn a_key_pause_reaches_its_sleeping_caller_past_many_paused_keys() {
    let (limits, _) = chat_limiter(per_second(10_000, 10_000));
    for chat in 0..5_000 {
        limits
            .penalize_for("chat_id", chat, Duration::from_secs(30))
            .await
            .unwrap();
    }
    limits
        .ready_for("chat_id", "target", None)
        .await
        .expect("the chat's first slot");
    let started = Instant::now();
    let sleeper = tokio::spawn({
        let limits = Arc::clone(&limits);
        async move { limits.ready_for("chat_id", "target", None).await }
    });
    // The sleeper books the chat's next slot, one second out.
    tokio::task::yield_now().await;
    limits
        .penalize_for("chat_id", "target", Duration::from_secs(20))
        .await
        .unwrap();
    sleeper.await.unwrap().expect("admitted after the pause");
    assert!(
        started.elapsed() >= Duration::from_secs(20),
        "woke at {:?}",
        started.elapsed()
    );
}

/// Every key keeps its own run of refusals, however many keys there are.
#[tokio::test(start_paused = true)]
async fn refusal_runs_are_exact_per_key() {
    let (limits, _) = chat_limiter(per_second(10_000, 10_000));
    let values: Vec<String> = (0..200).map(|chat| chat.to_string()).collect();
    for value in &values {
        limits
            .report(
                Verdict::KeyThrottled { retry_after: None },
                Some(("chat_id", value)),
            )
            .await;
    }
    {
        let runs = limits.key_refusals.lock().unwrap();
        assert_eq!(runs.len(), values.len());
        assert!(
            runs.values().all(|(count, _)| *count == 1),
            "no key shares a run"
        );
    }
    limits
        .report(Verdict::Pass, Some(("chat_id", &values[0])))
        .await;
    let runs = limits.key_refusals.lock().unwrap();
    assert_eq!(
        runs.len(),
        values.len() - 1,
        "a pass ends only that key's run"
    );
}

/// A deadline that has already passed refuses the call even when a slot is
/// free right away.
#[tokio::test(start_paused = true)]
async fn an_expired_deadline_refuses_an_immediate_slot() {
    let limits = limiter(per_second(10, 10));
    let expired = std::time::Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("an instant a millisecond ago");
    let error = limits
        .ready(Some(expired))
        .await
        .expect_err("past the deadline");
    assert!(
        matches!(error.kind(), ErrorKind::Exhausted { .. }),
        "{error}"
    );
}

/// Books like the in-memory store but cannot record a penalty.
struct PenaltyFails(MemoryLimitStore);

impl nebula_resilience::rate_limiter::gcra::LimitStore for PenaltyFails {
    async fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> Result<Result<Grant, Denied>, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::reserve(&self.0, key, rate, request)
            .await
    }

    async fn penalize(
        &self,
        _key: &LimitKey,
        _rate: &Rate,
        _retry_after: Duration,
        _max_penalty: Duration,
    ) -> Result<(), LimitStoreError> {
        Err(LimitStoreError::Unavailable("store down".into()))
    }

    async fn cancel(
        &self,
        key: &LimitKey,
        rate: &Rate,
        grant: &Grant,
    ) -> Result<bool, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::cancel(&self.0, key, rate, grant).await
    }

    async fn penalty(&self, key: &LimitKey) -> Result<Duration, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::penalty(&self.0, key).await
    }
}

/// The local pause is recorded before the store is asked, so a caller of
/// this process already sleeping honours it even when the store fails.
#[tokio::test(start_paused = true)]
async fn a_pause_holds_locally_when_the_store_cannot_record_it() {
    let limits = Arc::new(ResourceLimiter::new(
        Some(Quota::new(
            Arc::new(PenaltyFails(MemoryLimitStore::new())),
            LimitKey::new("test:row").unwrap(),
            per_second(1, 1),
        )),
        None,
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    ));
    limits.ready(None).await.expect("the burst passes");
    let started = Instant::now();
    let sleeper = tokio::spawn({
        let limits = Arc::clone(&limits);
        async move { limits.ready(None).await }
    });
    tokio::task::yield_now().await;
    limits
        .penalize(Duration::from_secs(20))
        .await
        .expect_err("the store is down");
    sleeper.await.unwrap().expect("admitted after the pause");
    assert!(
        started.elapsed() >= Duration::from_secs(20),
        "woke at {:?}",
        started.elapsed()
    );
}

/// A penalty another worker records after this caller booked holds the
/// caller back when it wakes: both limiters share one store, as two worker
/// processes share a cluster store.
#[tokio::test(start_paused = true)]
async fn a_penalty_from_another_worker_holds_back_a_booked_caller() {
    let store: Arc<dyn ErasedLimitStore> = Arc::new(MemoryLimitStore::new());
    let worker = || {
        Arc::new(ResourceLimiter::new(
            Some(Quota::new(
                Arc::clone(&store),
                LimitKey::new("acct:shared").unwrap(),
                per_second(1, 1),
            )),
            None,
            Duration::from_mins(5),
            ResourceKey::new("test.resource").unwrap(),
            Arc::new(EventBus::new(16)),
        ))
    };
    let (a, b) = (worker(), worker());
    b.ready(None).await.expect("the burst passes");
    let started = Instant::now();
    let booked = tokio::spawn({
        let b = Arc::clone(&b);
        async move { b.ready(None).await }
    });
    // B books its next slot, a second out, and sleeps.
    tokio::task::yield_now().await;
    a.penalize(Duration::from_mins(1))
        .await
        .expect("recorded in the shared store");
    booked.await.unwrap().expect("admitted after the penalty");
    assert!(
        started.elapsed() >= Duration::from_mins(1),
        "woke at {:?}",
        started.elapsed()
    );
}

/// A key's pause the store could not record is kept in this process until
/// it ends, for callers arriving after the throttled call returned.
#[tokio::test(start_paused = true)]
async fn an_unrecorded_key_pause_holds_for_later_callers() {
    let store: Arc<dyn ErasedLimitStore> = Arc::new(PenaltyFails(MemoryLimitStore::new()));
    let base = LimitKey::new("acct:test").unwrap();
    let limits = ResourceLimiter::new(
        Some(Quota::new(
            Arc::clone(&store),
            base.clone(),
            per_second(100, 100),
        )),
        Some(KeyedLimits::new(
            store,
            base,
            vec![("chat_id", per_second(100, 100))],
        )),
        Duration::from_mins(5),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    limits
        .penalize_for("chat_id", 7, Duration::from_secs(30))
        .await
        .expect_err("the store is down");
    let started = Instant::now();
    limits
        .ready_for("chat_id", 7, None)
        .await
        .expect("admitted after the pause");
    assert!(
        started.elapsed() >= Duration::from_secs(30),
        "woke at {:?}",
        started.elapsed()
    );
    let other = Instant::now();
    limits
        .ready_for("chat_id", 8, None)
        .await
        .expect("another chat is not paused");
    assert_eq!(other.elapsed(), Duration::ZERO);
}

/// Books like the in-memory store, but takes ten seconds to answer.
struct SlowStore(MemoryLimitStore);

impl nebula_resilience::rate_limiter::gcra::LimitStore for SlowStore {
    async fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> Result<Result<Grant, Denied>, LimitStoreError> {
        tokio::time::sleep(Duration::from_secs(10)).await;
        nebula_resilience::rate_limiter::gcra::LimitStore::reserve(&self.0, key, rate, request)
            .await
    }

    async fn penalize(
        &self,
        key: &LimitKey,
        rate: &Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> Result<(), LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::penalize(
            &self.0,
            key,
            rate,
            retry_after,
            max_penalty,
        )
        .await
    }

    async fn cancel(
        &self,
        key: &LimitKey,
        rate: &Rate,
        grant: &Grant,
    ) -> Result<bool, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::cancel(&self.0, key, rate, grant).await
    }

    async fn penalty(&self, key: &LimitKey) -> Result<Duration, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::penalty(&self.0, key).await
    }
}

/// A store slow to answer does not keep the caller past its deadline.
#[tokio::test(start_paused = true)]
async fn a_slow_store_does_not_hold_the_caller_past_its_deadline() {
    let limits = ResourceLimiter::new(
        Some(Quota::new(
            Arc::new(SlowStore(MemoryLimitStore::new())),
            LimitKey::new("test:row").unwrap(),
            per_second(10, 10),
        )),
        None,
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    let started = Instant::now();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let error = limits
        .ready(Some(deadline))
        .await
        .expect_err("the store answers too late");
    assert!(
        matches!(error.kind(), ErrorKind::Exhausted { .. }),
        "{error}"
    );
    assert!(
        started.elapsed() <= Duration::from_secs(2),
        "gave up at {:?}",
        started.elapsed()
    );
}

/// Books like the in-memory store, but never answers a penalty or a refund.
struct Hangs(MemoryLimitStore);

impl nebula_resilience::rate_limiter::gcra::LimitStore for Hangs {
    async fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> Result<Result<Grant, Denied>, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::reserve(&self.0, key, rate, request)
            .await
    }

    async fn penalize(
        &self,
        _key: &LimitKey,
        _rate: &Rate,
        _retry_after: Duration,
        _max_penalty: Duration,
    ) -> Result<(), LimitStoreError> {
        std::future::pending().await
    }

    async fn cancel(
        &self,
        _key: &LimitKey,
        _rate: &Rate,
        _grant: &Grant,
    ) -> Result<bool, LimitStoreError> {
        std::future::pending().await
    }

    async fn penalty(&self, key: &LimitKey) -> Result<Duration, LimitStoreError> {
        nebula_resilience::rate_limiter::gcra::LimitStore::penalty(&self.0, key).await
    }
}

fn hanging_chat_limiter() -> (ResourceLimiter, Arc<dyn ErasedLimitStore>) {
    let store: Arc<dyn ErasedLimitStore> = Arc::new(Hangs(MemoryLimitStore::new()));
    let base = LimitKey::new("acct:test").unwrap();
    let limits = ResourceLimiter::new(
        Some(Quota::new(
            Arc::clone(&store),
            base.clone(),
            per_second(100, 100),
        )),
        Some(KeyedLimits::new(
            Arc::clone(&store),
            base,
            vec![("chat_id", per_second(100, 100))],
        )),
        Duration::from_mins(5),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    (limits, store)
}

/// A key's pause is kept when the call recording it is cancelled while the
/// store write is pending: that write may never land.
#[tokio::test(start_paused = true)]
async fn a_key_pause_survives_a_cancelled_recording() {
    let (limits, _) = hanging_chat_limiter();
    let recording = tokio::time::timeout(
        Duration::from_millis(10),
        limits.penalize_for("chat_id", 7, Duration::from_secs(30)),
    )
    .await;
    assert!(recording.is_err(), "the store never answers");
    let started = Instant::now();
    limits
        .ready_for("chat_id", 7, None)
        .await
        .expect("admitted after the pause");
    assert!(
        started.elapsed() >= Duration::from_secs(29),
        "woke at {:?}",
        started.elapsed()
    );
}

/// Background refunds are bounded in number and in time, so a stalled
/// store cannot pile them up.
#[tokio::test(start_paused = true)]
async fn background_refunds_are_bounded() {
    let (limits, store) = hanging_chat_limiter();
    let key = LimitKey::new("acct:test").unwrap();
    let rate = per_second(100, 100);
    let grant = Grant {
        wait: Duration::ZERO,
        permits: 1,
        allow_at: 0,
        end_tat: 0,
        seq: 0,
    };
    for _ in 0..100 {
        limits.release(&store, &key, &rate, grant);
    }
    tokio::task::yield_now().await;
    assert_eq!(
        limits.refunds.load(Ordering::SeqCst),
        MAX_PENDING_REFUNDS,
        "at most the bound run at once; the rest lapse"
    );
    tokio::time::sleep(REFUND_BUDGET + Duration::from_millis(10)).await;
    assert_eq!(
        limits.refunds.load(Ordering::SeqCst),
        0,
        "each is abandoned after its budget"
    );
}

/// Once a client is wrapped, its calls book the quota and an acquire only
/// honours pauses: one provider call uses one permit, not two.
#[tokio::test(start_paused = true)]
async fn a_wrapped_limit_is_booked_per_call_not_per_acquire() {
    let limits = Arc::new(limiter(per_second(1, 1)));
    let client = limits.wrap((), NoThrottle);
    let started = Instant::now();
    for _ in 0..3 {
        limits
            .ready_to_acquire(None)
            .await
            .expect("acquire books nothing");
    }
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("the call books its own slot");
    assert_eq!(
        started.elapsed(),
        Duration::ZERO,
        "one permit, used by the call"
    );
}

/// The first acquire of a resource that wraps its client while creating
/// books the permit its first call then uses: one provider call, one
/// permit, even on a cold start.
#[tokio::test(start_paused = true)]
async fn a_cold_acquire_and_its_first_call_share_one_permit() {
    let limits = Arc::new(limiter(per_second(1, 1)));
    limits
        .ready_to_acquire(None)
        .await
        .expect("the cold acquire books");
    let client = limits.wrap((), NoThrottle);
    let started = Instant::now();
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("uses the acquire's permit");
    assert_eq!(
        started.elapsed(),
        Duration::ZERO,
        "no second permit waited for"
    );
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("the next call books its own");
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}

/// Two workers sharing one account quota, as two processes share a cluster
/// store.
fn shared_account_workers() -> (Arc<ResourceLimiter>, Arc<ResourceLimiter>) {
    let store: Arc<dyn ErasedLimitStore> = Arc::new(MemoryLimitStore::new());
    let worker = || {
        Arc::new(ResourceLimiter::new(
            Some(Quota::new(
                Arc::clone(&store),
                LimitKey::new("acct:shared").unwrap(),
                per_second(1, 1),
            )),
            None,
            Duration::from_mins(5),
            ResourceKey::new("test.resource").unwrap(),
            Arc::new(EventBus::new(16)),
        ))
    };
    (worker(), worker())
}

/// The credit a cold acquire leaves for its first call is spent only after
/// the store's penalties are read: the client is built between the two, and
/// a `Retry-After` another worker saw meanwhile holds this call back as it
/// holds every other caller of the account.
#[tokio::test(start_paused = true)]
async fn a_prepaid_permit_honours_a_penalty_another_worker_recorded() {
    let (a, b) = shared_account_workers();
    b.ready_to_acquire(None)
        .await
        .expect("the cold acquire books");
    // While B builds its client, A is told to slow down.
    a.penalize(Duration::from_mins(1))
        .await
        .expect("recorded in the shared store");
    let client = b.wrap((), NoThrottle);
    let started = Instant::now();
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("admitted once the penalty ends");
    assert!(
        started.elapsed() >= Duration::from_mins(1),
        "ran at {:?}",
        started.elapsed()
    );
}

/// A pause forfeits the prepaid slot: after it the call books again, so it
/// does not run in the same slot as a caller that booked the first slot
/// after the pause.
#[tokio::test(start_paused = true)]
async fn a_prepaid_permit_rebooks_after_a_shared_penalty() {
    let (a, b) = shared_account_workers();
    b.ready_to_acquire(None)
        .await
        .expect("the cold acquire books");
    a.penalize(Duration::from_mins(1))
        .await
        .expect("recorded in the shared store");
    let started = Instant::now();
    // Another worker books the first slot after the penalty.
    let other = tokio::spawn({
        let a = Arc::clone(&a);
        async move {
            a.ready(None).await.expect("admitted after the penalty");
            Instant::now()
        }
    });
    tokio::task::yield_now().await;
    let client = b.wrap((), NoThrottle);
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("admitted after the penalty");
    let prepaid_at = Instant::now();
    let other_at = other.await.unwrap();
    assert!(
        prepaid_at.duration_since(started) >= Duration::from_mins(1),
        "ran at {:?}",
        prepaid_at.duration_since(started)
    );
    assert!(
        prepaid_at.max(other_at) - prepaid_at.min(other_at) >= Duration::from_secs(1),
        "the two calls share one account slot: {:?} and {:?}",
        prepaid_at.duration_since(started),
        other_at.duration_since(started)
    );
}

/// The credit never carries a call past its deadline: a penalty that
/// outlasts the deadline fails fast, and the call does not run.
#[tokio::test(start_paused = true)]
async fn a_prepaid_permit_fails_fast_when_a_penalty_outlasts_its_deadline() {
    let (a, b) = shared_account_workers();
    b.ready_to_acquire(None)
        .await
        .expect("the cold acquire books");
    a.penalize(Duration::from_mins(1))
        .await
        .expect("recorded in the shared store");
    let client = b.wrap((), NoThrottle);
    let ran = Arc::new(AtomicBool::new(false));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let error = client
        .run_until(Some(deadline), async |()| {
            ran.store(true, Ordering::Relaxed);
            Ok::<_, ProviderError>(())
        })
        .await
        .expect_err("the penalty outlasts the deadline");
    assert!(
        matches!(
            &error,
            LimitedError::Limit(error) if matches!(error.kind(), ErrorKind::Exhausted { .. })
        ),
        "{error:?}"
    );
    assert!(!ran.load(Ordering::Relaxed), "the call must not run");
}

/// A keyed first call books the account itself and drops the cold
/// acquire's credit, so a following unkeyed call cannot reuse it and run in
/// the same account slot.
#[tokio::test(start_paused = true)]
async fn a_keyed_first_call_drops_the_cold_acquire_credit() {
    let (limits, _) = chat_limiter(per_second(1, 1));
    limits
        .ready_to_acquire(None)
        .await
        .expect("the cold acquire books");
    let client = limits.wrap((), NoThrottle);
    let started = Instant::now();
    client
        .run_for("chat_id", 1, async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("keyed call");
    let keyed_at = started.elapsed();
    client
        .run(async |()| Ok::<_, ProviderError>(()))
        .await
        .expect("unkeyed call");
    assert!(
        started.elapsed().saturating_sub(keyed_at) >= Duration::from_secs(1),
        "the unkeyed call takes its own account slot: {keyed_at:?} then {:?}",
        started.elapsed()
    );
}

/// Once a client is wrapped, an acquire still never passes its deadline.
#[tokio::test(start_paused = true)]
async fn a_wrapped_acquire_keeps_its_deadline() {
    let limits = Arc::new(limiter(per_second(1, 1)));
    let _client = limits.wrap((), NoThrottle);
    let expired = std::time::Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("an instant a millisecond ago");
    let error = limits
        .ready_to_acquire(Some(expired))
        .await
        .expect_err("past the deadline");
    assert!(
        matches!(error.kind(), ErrorKind::Exhausted { .. }),
        "{error}"
    );
}

/// Callers that booked before a penalty keep their spacing after it: each
/// books again once the pause is over instead of all running at its end.
#[tokio::test(start_paused = true)]
async fn callers_behind_a_penalty_keep_their_spacing() {
    let limits = Arc::new(limiter(per_second(1, 1)));
    limits.ready(None).await.expect("the burst passes");
    let started = Instant::now();
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let limits = Arc::clone(&limits);
            tokio::spawn(async move {
                limits.ready(None).await.expect("admitted");
                started.elapsed()
            })
        })
        .collect();
    tokio::task::yield_now().await;
    limits
        .penalize(Duration::from_secs(5))
        .await
        .expect("recorded");
    let mut done = Vec::new();
    for caller in callers {
        done.push(caller.await.unwrap());
    }
    done.sort();
    assert!(done[0] >= Duration::from_secs(5), "{done:?}");
    assert!(
        done[1].saturating_sub(done[0]) >= Duration::from_millis(999),
        "still a second apart after the pause: {done:?}"
    );
}

/// Account and key slots align even when both intervals are equal: a
/// replaced slot still in the future is returned before the rebook, so the
/// two meet within a round or two instead of chasing each other until the
/// alignment gives up. (A slot already due cannot be returned, so the
/// meeting point may be one interval later than ideal: never earlier.)
#[tokio::test(start_paused = true)]
async fn equal_account_and_key_intervals_still_align() {
    let (limits, _) = chat_limiter(per_second(1, 1));
    limits.ready(None).await.expect("the account's first slot");
    tokio::time::advance(Duration::from_millis(500)).await;
    let started = Instant::now();
    limits
        .ready_for("chat_id", 42, None)
        .await
        .expect("both limits agree on a slot");
    assert!(
        started.elapsed() >= Duration::from_millis(500)
            && started.elapsed() <= Duration::from_secs(1),
        "admitted at {:?}",
        started.elapsed()
    );
    // And the next call for the chat still keeps both limits' spacing.
    let first = started.elapsed();
    limits
        .ready_for("chat_id", 42, None)
        .await
        .expect("the next slot");
    assert!(
        started.elapsed().saturating_sub(first) >= Duration::from_secs(1),
        "a second apart"
    );
}

/// When a full in-process store puts both the account and the key on its
/// shared overflow limit, alignment cannot meet on one slot; the call still
/// goes through instead of failing every time.
#[tokio::test(start_paused = true)]
async fn alignment_on_a_full_store_still_admits() {
    let store = Arc::new(MemoryLimitStore::with_max_keys(1));
    nebula_resilience::rate_limiter::gcra::LimitStore::reserve(
        &*store,
        &LimitKey::new("occupant").unwrap(),
        &Rate::new(nz(1), Duration::from_hours(1)).unwrap(),
        ReserveRequest::new(1, Duration::ZERO),
    )
    .await
    .unwrap()
    .unwrap();
    let shared: Arc<dyn ErasedLimitStore> = store;
    let base = LimitKey::new("acct:test").unwrap();
    let limits = ResourceLimiter::new(
        Some(Quota::new(
            Arc::clone(&shared),
            base.clone(),
            per_second(1, 1),
        )),
        Some(KeyedLimits::new(
            shared,
            base,
            vec![("chat_id", per_second(1, 1))],
        )),
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    );
    for _ in 0..3 {
        limits
            .ready_for(
                "chat_id",
                7,
                Some(std::time::Instant::now() + Duration::from_secs(30)),
            )
            .await
            .expect("admitted on the overflow limit");
    }
}

/// With per-key limits but no account rate, callers behind an account
/// pause keep their key's spacing after it instead of all running as it
/// ends.
#[tokio::test(start_paused = true)]
async fn key_calls_behind_an_account_pause_keep_their_spacing() {
    let limits = Arc::new(ResourceLimiter::new(
        None,
        Some(KeyedLimits::new(
            Arc::new(MemoryLimitStore::new()),
            LimitKey::new("acct:test").unwrap(),
            vec![("chat_id", per_second(1, 1))],
        )),
        Duration::from_mins(1),
        ResourceKey::new("test.resource").unwrap(),
        Arc::new(EventBus::new(16)),
    ));
    limits
        .penalize(Duration::from_secs(5))
        .await
        .expect("local pause");
    let started = Instant::now();
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let limits = Arc::clone(&limits);
            tokio::spawn(async move {
                limits
                    .ready_for("chat_id", 7, None)
                    .await
                    .expect("admitted");
                started.elapsed()
            })
        })
        .collect();
    let mut done = Vec::new();
    for caller in callers {
        done.push(caller.await.unwrap());
    }
    done.sort();
    assert!(done[0] >= Duration::from_secs(5), "{done:?}");
    assert!(
        done[1].saturating_sub(done[0]) >= Duration::from_millis(999),
        "the chat's calls stay a second apart after the pause: {done:?}"
    );
}
