use std::{num::NonZeroU32, time::Duration};

use proptest::prelude::*;

use super::*;
use crate::rate_limiter::RateLimiter as _;

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("non-zero test value")
}

fn rate(interval_ns: u64, burst: u32) -> Rate {
    Rate::new(nz(1), Duration::from_nanos(interval_ns))
        .unwrap()
        .with_burst(nz(burst))
        .unwrap()
}

/// Largest number of permits used inside any half-open window of `len`
/// nanoseconds, brute force over window starts at every use time.
fn max_in_window(uses: &[u64], len: u64) -> usize {
    uses.iter()
        .map(|start| {
            uses.iter()
                .filter(|used| **used >= *start && **used < start.saturating_add(len))
                .count()
        })
        .max()
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
enum Op {
    Advance(u64),
    Reserve { permits: u32, max_wait: Duration },
}

fn ops(burst: u32) -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        (0_u64..2_000).prop_map(Op::Advance),
        (
            0..=burst + 1,
            prop_oneof![
                Just(Duration::ZERO),
                (1_u64..3_000).prop_map(Duration::from_nanos),
                Just(Duration::MAX),
            ]
        )
            .prop_map(|(permits, max_wait)| Op::Reserve { permits, max_wait }),
    ];
    proptest::collection::vec(op, 1..200)
}

/// Runs `ops` and returns the use time of every granted permit, checking the
/// per-step contract on the way.
fn run(rate: &Rate, ops: &[Op]) -> Vec<u64> {
    let mut state = GcraState::default();
    let mut now = 0_u64;
    let mut uses = Vec::new();
    let mut last_allow = 0_u64;
    for op in ops {
        match *op {
            Op::Advance(by) => now += by,
            Op::Reserve { permits, max_wait } => {
                let (decision, next) = step::reserve(state, now, rate, permits, max_wait);
                match decision {
                    Ok(grant) => {
                        assert_eq!(grant.allow_at, now + nanos(grant.wait));
                        assert!(grant.wait <= max_wait, "never waits past max_wait");
                        if permits > 0 {
                            assert!(grant.allow_at >= last_allow, "grants are FIFO");
                            last_allow = grant.allow_at;
                            uses.extend(std::iter::repeat_n(grant.allow_at, permits as usize));
                        }
                    },
                    Err(Denied::Never { burst }) => {
                        assert!(permits > burst);
                        assert!(next.is_none());
                    },
                    Err(Denied::Later { retry_after }) => {
                        assert!(next.is_none(), "a refusal consumes nothing");
                        assert!(!retry_after.is_zero());
                    },
                }
                if let Some(next) = next {
                    state = next;
                }
            },
        }
    }
    uses
}

proptest! {
    /// GCRA's defining bound: at most `B + ceil(L / T) − 1` permits used in
    /// any window of length `L`, whatever the arrival pattern.
    #[test]
    fn never_exceeds_the_gcra_bound(
        interval in 1_u64..500,
        burst in 1_u32..6,
        window_intervals in 1_u64..20,
        ops in ops(5),
    ) {
        let rate = rate(interval, burst);
        let uses = run(&rate, &ops);
        let len = interval * window_intervals;
        let bound = u64::from(burst) + len.div_ceil(interval) - 1;
        prop_assert!(max_in_window(&uses, len) as u64 <= bound);
    }

    /// `per_window(N, W, B)` never lets more than `N` permits into a window
    /// of `W`.
    #[test]
    fn per_window_keeps_every_window_within_the_quota(
        limit in 1_u32..40,
        burst_seed in 1_u32..40,
        window in 50_u64..5_000,
        ops in ops(40),
    ) {
        let burst = burst_seed.min(limit);
        let rate = Rate::per_window(nz(limit), Duration::from_nanos(window), nz(burst), 0).unwrap();
        let uses = run(&rate, &ops);
        prop_assert!(max_in_window(&uses, window) <= limit as usize);
    }
}

#[test]
fn interval_is_rounded_up_so_the_rate_is_never_exceeded() {
    let rate = Rate::new(nz(3), Duration::from_secs(1)).unwrap();
    assert_eq!(rate.emission_interval(), Duration::from_nanos(333_333_334));
    assert_eq!(
        Rate::per_second(nz(3)).emission_interval(),
        rate.emission_interval()
    );
}

#[test]
fn invalid_rates_are_config_errors() {
    assert!(Rate::new(nz(1), Duration::ZERO).is_err());
    assert!(Rate::new(nz(1), Duration::from_secs(u64::MAX)).is_err());
    assert!(
        Rate::new(nz(1), Duration::from_secs(u64::MAX / 2_000_000_000))
            .unwrap()
            .with_burst(nz(u32::MAX))
            .is_err(),
        "a burst window past u64 nanoseconds is refused, not wrapped"
    );
    assert!(Rate::per_window(nz(10), Duration::from_mins(1), nz(11), 0).is_err());
    assert!(Rate::per_window(nz(10), Duration::from_mins(1), nz(1), 100).is_err());
    assert!(Rate::try_from(RateConfig::new(0, 1_000)).is_err());
    assert!(Rate::try_from(RateConfig::new(1, 0)).is_err());
    assert!(Rate::try_from(RateConfig::new(1, 1_000).with_burst(0)).is_err());
}

#[test]
fn per_window_holds_back_the_safety_margin() {
    // 60/min, 10 % margin: 54 effective, burst 1 -> one every 60/54 s.
    let rate = Rate::per_window(nz(60), Duration::from_mins(1), nz(1), 10).unwrap();
    assert_eq!(
        rate.emission_interval(),
        Duration::from_nanos(1_111_111_112)
    );
}

#[test]
fn tighter_means_a_longer_interval_and_a_smaller_burst() {
    let declared = Rate::per_second(nz(30)).with_burst(nz(30)).unwrap();
    let slower = Rate::per_second(nz(10)).with_burst(nz(10)).unwrap();
    // Same count per period over a shorter period is *looser*, not tighter.
    let shorter_period = Rate::new(nz(30), Duration::from_millis(500)).unwrap();
    assert!(slower.is_no_looser_than(&declared));
    assert!(!declared.is_no_looser_than(&slower));
    assert!(!shorter_period.is_no_looser_than(&declared));
}

#[test]
fn penalty_blocks_until_retry_after_and_is_capped() {
    let rate = rate(10, 3);
    let state = step::penalize(
        GcraState::default(),
        100,
        &rate,
        Duration::from_micros(1),
        Duration::from_nanos(500),
    );
    let (early, _) = step::reserve(state, 599, &rate, 1, Duration::ZERO);
    assert!(
        matches!(early, Err(Denied::Later { .. })),
        "blocked until the capped penalty"
    );
    let (on_time, _) = step::reserve(state, 600, &rate, 1, Duration::ZERO);
    assert!(
        on_time.is_ok(),
        "free again once the capped penalty elapsed"
    );
    // Monotonic: a smaller penalty never releases a larger one.
    let smaller = step::penalize(state, 100, &rate, Duration::from_nanos(10), Duration::MAX);
    assert_eq!(smaller.tat, state.tat);
}

#[test]
fn only_the_tail_reservation_is_refunded_and_only_once() {
    let rate = rate(100, 1);
    let (first, state) = step::reserve(GcraState::default(), 0, &rate, 1, Duration::MAX);
    let (first, state) = (first.unwrap(), state.unwrap());
    let (second, after_second) = step::reserve(state, 0, &rate, 1, Duration::MAX);
    let (second, after_second) = (second.unwrap(), after_second.unwrap());
    assert!(
        step::cancel(after_second, 0, &rate, &first).is_none(),
        "a reservation with a later one behind it is not refunded"
    );
    let refunded = step::cancel(after_second, 0, &rate, &second).expect("tail is refunded");
    assert_eq!(refunded.tat, state.tat);
    assert!(
        step::cancel(refunded, 0, &rate, &second).is_none(),
        "a repeated cancel is a no-op"
    );
    // ABA: a new reservation that lands on the same TAT is not the old one.
    let (third, after_third) = step::reserve(refunded, 0, &rate, 1, Duration::MAX);
    let after_third = after_third.unwrap();
    assert_eq!(third.unwrap().end_tat, second.end_tat);
    assert!(step::cancel(after_third, 0, &rate, &second).is_none());
}

#[test]
fn a_slot_that_has_arrived_is_not_refunded() {
    let rate = rate(100, 1);
    let (grant, state) = step::reserve(GcraState::default(), 0, &rate, 1, Duration::MAX);
    let (grant, state) = (grant.unwrap(), state.unwrap());
    assert!(
        step::cancel(state, 0, &rate, &grant).is_none(),
        "wait was zero: already used"
    );
}

#[test]
fn available_counts_the_remaining_burst() {
    let rate = rate(100, 3);
    let mut state = GcraState::default();
    assert_eq!(step::available(state, 0, &rate), 3);
    state = step::reserve(state, 0, &rate, 2, Duration::ZERO).1.unwrap();
    assert_eq!(step::available(state, 0, &rate), 1);
    assert_eq!(step::available(state, 200, &rate), 3);
}

#[tokio::test(start_paused = true)]
async fn limiter_waits_to_the_deadline_and_fails_fast_beyond_it() {
    let limiter = Gcra::new(Rate::per_second(nz(2)));
    let start = tokio::time::Instant::now();
    limiter.until_ready(1, None).await.unwrap();
    limiter
        .until_ready(1, Some(start + Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(start.elapsed(), Duration::from_millis(500));

    let before = tokio::time::Instant::now();
    let denied = limiter
        .until_ready(1, Some(before + Duration::from_millis(100)))
        .await
        .unwrap_err();
    assert!(matches!(denied, Denied::Later { .. }));
    assert_eq!(before.elapsed(), Duration::ZERO, "no waiting on a refusal");
}

#[tokio::test(start_paused = true)]
async fn acquire_keeps_the_fail_fast_contract() {
    let limiter = Gcra::new(Rate::per_second(nz(1)));
    limiter.acquire().await.unwrap();
    let error = limiter.acquire().await.unwrap_err();
    assert!(matches!(
        error,
        crate::CallError::RateLimited {
            retry_after: Some(_)
        }
    ));
    tokio::time::advance(Duration::from_secs(1)).await;
    limiter.acquire().await.unwrap();
    assert_eq!(limiter.available(), 0);
    limiter.reset().await;
    assert_eq!(limiter.available(), 1);
    let status = limiter.status().await;
    assert!((status.remaining - 1.0).abs() < f64::EPSILON);
    assert_eq!(status.limit_per_second.map(f64::round), Some(1.0));
}

#[tokio::test(start_paused = true)]
async fn memory_store_keys_are_independent_and_idle_keys_vanish() {
    let store = MemoryLimitStore::new();
    let rate = Rate::per_second(nz(1));
    let a = LimitKey::new("tenant:a").unwrap();
    let b = LimitKey::new("tenant:b").unwrap();
    let now = ReserveRequest::new(1, Duration::ZERO);
    assert!(store.reserve(&a, &rate, now).await.unwrap().is_ok());
    assert!(store.reserve(&a, &rate, now).await.unwrap().is_err());
    assert!(
        store.reserve(&b, &rate, now).await.unwrap().is_ok(),
        "keys do not share a limit"
    );
    assert_eq!(store.len(), 2);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(
        store
            .reserve(&a, &rate, ReserveRequest::new(0, Duration::ZERO))
            .await
            .unwrap()
            .is_ok()
    );
    assert_eq!(store.len(), 1, "an idle key holds no state");
}

#[tokio::test(start_paused = true)]
async fn memory_store_reservations_are_idempotent_and_cancellable() {
    let store = MemoryLimitStore::new();
    let rate = Rate::per_second(nz(1));
    let key = LimitKey::new("throttle").unwrap();
    store
        .reserve(&key, &rate, ReserveRequest::new(1, Duration::ZERO))
        .await
        .unwrap()
        .unwrap();

    let request = ReserveRequest::new(1, Duration::MAX).with_id(ReservationId(7));
    let booked = store.reserve(&key, &rate, request).await.unwrap().unwrap();
    assert_eq!(booked.wait, Duration::from_secs(1));
    tokio::time::advance(Duration::from_millis(400)).await;
    let replay = store.reserve(&key, &rate, request).await.unwrap().unwrap();
    assert_eq!(
        replay.end_tat, booked.end_tat,
        "a replay returns the original grant"
    );
    assert_eq!(replay.wait, Duration::from_millis(600));

    assert!(store.cancel(&key, &rate, &booked).await.unwrap());
    assert!(
        !store.cancel(&key, &rate, &booked).await.unwrap(),
        "cancel is idempotent"
    );
    let fresh = store.reserve(&key, &rate, request).await.unwrap().unwrap();
    assert_eq!(
        fresh.wait,
        Duration::from_millis(600),
        "the refunded slot is free again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_store_passes_the_store_conformance_kit() {
    conformance::run_all(std::sync::Arc::new(MemoryLimitStore::new())).await;
}

#[tokio::test(start_paused = true)]
async fn an_idle_key_forgets_a_stricter_rate() {
    let store = MemoryLimitStore::new();
    let key = LimitKey::new("idle:forgets").unwrap();
    let strict = rate(1_000_000_000, 1);
    let loose = rate(1_000_000_000, 10);
    store
        .reserve(&key, &strict, ReserveRequest::new(1, Duration::ZERO))
        .await
        .unwrap()
        .unwrap();
    // Busy: the loose caller is held to the strict burst of one.
    assert!(
        store
            .reserve(&key, &loose, ReserveRequest::new(1, Duration::ZERO))
            .await
            .unwrap()
            .is_err()
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    // Idle: the loose caller's burst applies again.
    for _ in 0..10 {
        store
            .reserve(&key, &loose, ReserveRequest::new(1, Duration::ZERO))
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_full_memory_store_shares_one_stricter_limit_for_new_keys() {
    let store = MemoryLimitStore::with_max_keys(2);
    let one_per_hour = rate(3_600_000_000_000, 1);
    let reserve = |name: &str| {
        let key = LimitKey::new(name).unwrap();
        let store = &store;
        async move {
            store
                .reserve(&key, &one_per_hour, ReserveRequest::new(1, Duration::ZERO))
                .await
                .unwrap()
        }
    };
    assert!(reserve("a").await.is_ok());
    assert!(reserve("b").await.is_ok());
    assert_eq!(store.len(), 2);
    // Full and nothing idle: new keys share the overflow limit.
    assert!(reserve("c").await.is_ok(), "the overflow limit starts free");
    assert!(
        reserve("d").await.is_err(),
        "another new key shares c's spent overflow limit"
    );
    assert_eq!(store.len(), 2, "the store never grows past its bound");
    assert_eq!(store.overflowed(), 2);
    // Two overflowed keys reusing one reservation id are two reservations,
    // not a replay of each other.
    let id = ReservationId(7);
    let request = ReserveRequest::new(1, Duration::MAX).with_id(id);
    let first = store
        .reserve(&LimitKey::new("g").unwrap(), &one_per_hour, request)
        .await
        .unwrap()
        .unwrap();
    let second = store
        .reserve(&LimitKey::new("h").unwrap(), &one_per_hour, request)
        .await
        .unwrap()
        .unwrap();
    assert!(
        second.allow_at > first.allow_at,
        "the second books its own slot"
    );
    // Once keys go idle, new keys get their own entries again.
    tokio::time::advance(Duration::from_hours(2)).await;
    assert!(reserve("e").await.is_ok());
    assert!(reserve("f").await.is_ok());
    assert_eq!(store.overflowed(), 4, "only c, d, g and h overflowed");
}

#[test]
fn a_clock_that_steps_back_never_loosens_the_limit() {
    let limit = rate(1_000, 2);
    let (_, state) = step::reserve(GcraState::default(), 10_000, &limit, 2, Duration::ZERO);
    let state = state.unwrap();
    // The store clock steps back by far more than an interval: the booked
    // schedule still holds, so the next permit waits as long as before.
    let (early, _) = step::reserve(state, 5_000, &limit, 1, Duration::MAX);
    let (on_time, _) = step::reserve(state, 10_000, &limit, 1, Duration::MAX);
    let (early, on_time) = (early.unwrap(), on_time.unwrap());
    assert_eq!(early.allow_at, on_time.allow_at);
    assert!(early.wait >= on_time.wait);
}

#[test]
fn stricter_takes_the_longer_interval_and_the_smaller_burst() {
    let fast_burst = rate(1_000, 10);
    let slow_single = rate(5_000, 1);
    let strict = fast_burst.stricter(&slow_single);
    assert_eq!(strict.emission_interval(), Duration::from_micros(5));
    assert_eq!(strict.burst(), nz(1));
    assert!(strict.is_no_looser_than(&fast_burst) && strict.is_no_looser_than(&slow_single));
    assert_eq!(
        Rate::from_interval(Duration::from_micros(5), nz(3)).unwrap(),
        rate(5_000, 3)
    );
    assert!(Rate::from_interval(Duration::ZERO, nz(1)).is_err());
}

#[test]
fn limit_keys_are_bounded_printable_ascii() {
    assert!(LimitKey::new("rate:telegram:abc").is_ok());
    assert!(LimitKey::new("").is_err());
    assert!(LimitKey::new("has space").is_err());
    assert!(LimitKey::new("x".repeat(MAX_LIMIT_KEY_BYTES + 1)).is_err());
}

#[cfg(feature = "serde")]
#[test]
fn rate_config_rejects_unknown_fields() {
    let config: RateConfig = serde_json::from_value(
        serde_json::json!({ "requests": 30, "period_ms": 1000, "burst": 30 }),
    )
    .unwrap();
    assert_eq!(Rate::try_from(config).unwrap().burst(), nz(30));
    assert!(
        serde_json::from_value::<RateConfig>(serde_json::json!({
            "requests": 30, "period_ms": 1000, "per_minute": 1
        }))
        .is_err()
    );
}
