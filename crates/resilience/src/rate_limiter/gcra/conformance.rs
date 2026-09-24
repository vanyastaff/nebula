//! Behaviour every [`LimitStore`] must show, as a test kit.
//!
//! Enabled by the `conformance` feature for the test suites of store
//! implementations (the in-process store here, the database stores in
//! `nebula-storage`); like any test oracle it reports a violation by
//! panicking. [`run_all`] runs every case.
//!
//! The cases use rates of one permit per hour, so they decide the same way
//! on a real clock (a database) as on a paused one: nothing in them depends
//! on how many milliseconds a call takes. Each case works on fresh keys, so
//! the cases can share one store and run against a persistent database.
#![expect(
    clippy::missing_panics_doc,
    reason = "every case reports a contract violation by panicking, as the module states"
)]

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use super::{Denied, Grant, LimitKey, LimitStore, Rate, ReservationId, ReserveRequest};

const HOUR: Duration = Duration::from_hours(1);
/// Slack for the real time a case takes between two store calls.
const SLACK: Duration = Duration::from_mins(1);

/// One permit per hour, `burst` back to back.
fn hourly(burst: u32) -> Rate {
    let burst = NonZeroU32::new(burst).unwrap_or(NonZeroU32::MIN);
    Rate::new(NonZeroU32::MIN, HOUR)
        .and_then(|rate| rate.with_burst(burst))
        .unwrap_or_else(|error| panic!("test rate is valid: {error}"))
}

/// A key no other case or run uses.
fn fresh_key(case: &str) -> LimitKey {
    LimitKey::new(format!("conformance:{case}:{:016x}", fastrand::u64(..)))
        .unwrap_or_else(|error| panic!("test key is valid: {error}"))
}

async fn reserve<S: LimitStore>(
    store: &S,
    key: &LimitKey,
    rate: &Rate,
    request: ReserveRequest,
) -> Result<Grant, Denied> {
    store
        .reserve(key, rate, request)
        .await
        .unwrap_or_else(|error| panic!("store answers: {error}"))
}

async fn granted<S: LimitStore>(
    store: &S,
    key: &LimitKey,
    rate: &Rate,
    request: ReserveRequest,
) -> Grant {
    reserve(store, key, rate, request)
        .await
        .unwrap_or_else(|denied| panic!("expected a grant, got {denied:?}"))
}

async fn refused_for<S: LimitStore>(store: &S, key: &LimitKey, rate: &Rate) -> Duration {
    match reserve(store, key, rate, ReserveRequest::new(1, Duration::ZERO)).await {
        Err(Denied::Later { retry_after }) => retry_after,
        other => panic!("expected a refusal with a retry_after, got {other:?}"),
    }
}

fn assert_about(actual: Duration, expected: Duration, what: &str) {
    assert!(
        actual <= expected && actual + SLACK >= expected,
        "{what}: expected about {expected:?}, got {actual:?}"
    );
}

/// The burst passes at once; the next permit is refused with the time until
/// its slot, and a refusal consumes nothing.
pub async fn burst_then_refusal_consumes_nothing<S: LimitStore>(store: &S) {
    let key = fresh_key("burst");
    let rate = hourly(3);
    for _ in 0..3 {
        let grant = granted(store, &key, &rate, ReserveRequest::new(1, Duration::ZERO)).await;
        assert_eq!(grant.wait, Duration::ZERO, "the burst waits for nothing");
    }
    assert_about(refused_for(store, &key, &rate).await, HOUR, "first refusal");
    assert_about(
        refused_for(store, &key, &rate).await,
        HOUR,
        "a refusal consumed nothing",
    );
    let booked = granted(store, &key, &rate, ReserveRequest::new(1, Duration::MAX)).await;
    assert_about(booked.wait, HOUR, "the next slot can be booked ahead");
}

/// Keys never share state.
pub async fn keys_are_isolated<S: LimitStore>(store: &S) {
    let (spent, other) = (fresh_key("isolated-a"), fresh_key("isolated-b"));
    let rate = hourly(1);
    granted(store, &spent, &rate, ReserveRequest::new(1, Duration::ZERO)).await;
    refused_for(store, &spent, &rate).await;
    granted(store, &other, &rate, ReserveRequest::new(1, Duration::ZERO)).await;
}

/// More permits than the burst are never granted, however long one waits.
pub async fn permits_beyond_the_burst_are_never_granted<S: LimitStore>(store: &S) {
    let key = fresh_key("never");
    let decision = reserve(
        store,
        &key,
        &hourly(2),
        ReserveRequest::new(3, Duration::MAX),
    )
    .await;
    assert_eq!(decision, Err(Denied::Never { burst: 2 }));
}

/// A penalty blocks even an idle key, for at most the cap.
pub async fn penalty_blocks_an_idle_key_up_to_the_cap<S: LimitStore>(store: &S) {
    let key = fresh_key("penalty");
    let rate = hourly(1);
    let cap = Duration::from_mins(10);
    store
        .penalize(&key, &rate, HOUR, cap)
        .await
        .unwrap_or_else(|error| panic!("store answers: {error}"));
    assert_about(refused_for(store, &key, &rate).await, cap, "capped penalty");
}

/// Only the most recent reservation is refunded, and only once.
pub async fn cancel_refunds_only_the_tail_once<S: LimitStore>(store: &S) {
    let key = fresh_key("cancel");
    let rate = hourly(1);
    let now = granted(store, &key, &rate, ReserveRequest::new(1, Duration::ZERO)).await;
    let later = granted(store, &key, &rate, ReserveRequest::new(1, Duration::MAX)).await;
    let cancel = |grant: Grant| {
        let (key, rate) = (key.clone(), rate);
        async move {
            store
                .cancel(&key, &rate, &grant)
                .await
                .unwrap_or_else(|error| panic!("store answers: {error}"))
        }
    };
    assert!(!cancel(now).await, "not the tail, and its slot has passed");
    assert!(cancel(later).await, "the tail is refunded");
    assert!(!cancel(later).await, "a repeated cancel refunds nothing");
    let again = granted(store, &key, &rate, ReserveRequest::new(1, Duration::MAX)).await;
    assert_about(again.wait, HOUR, "the refunded slot is free again");
}

/// Repeating a reservation id returns the original grant and books nothing.
pub async fn repeated_reservation_id_returns_the_original_grant<S: LimitStore>(store: &S) {
    let key = fresh_key("idempotent");
    let rate = hourly(1);
    granted(store, &key, &rate, ReserveRequest::new(1, Duration::ZERO)).await;
    let id = ReservationId(u128::from(fastrand::u64(..)));
    let request = ReserveRequest::new(1, Duration::MAX).with_id(id);
    let first = granted(store, &key, &rate, request).await;
    let repeat = granted(store, &key, &rate, request).await;
    assert_eq!(
        (repeat.allow_at, repeat.end_tat, repeat.seq),
        (first.allow_at, first.end_tat, first.seq),
        "the repeat is the same reservation"
    );
    let next = granted(store, &key, &rate, ReserveRequest::new(1, Duration::MAX)).await;
    assert_about(next.wait, 2 * HOUR, "the repeat booked no second slot");
}

/// Callers that disagree on a busy key's rate get the stricter one.
pub async fn a_busy_key_enforces_the_stricter_rate<S: LimitStore>(store: &S) {
    let key = fresh_key("stricter");
    // One caller allows two back to back, another a hundred.
    granted(
        store,
        &key,
        &hourly(2),
        ReserveRequest::new(1, Duration::ZERO),
    )
    .await;
    let loose = hourly(100);
    granted(store, &key, &loose, ReserveRequest::new(1, Duration::ZERO)).await;
    assert_about(
        refused_for(store, &key, &loose).await,
        HOUR,
        "the looser caller is held to the stricter burst",
    );
}

/// A busy key tightening its interval stretches what was booked under the
/// looser one: the stricter rate also covers bookings made before it.
pub async fn tightening_a_busy_key_rebases_its_schedule<S: LimitStore>(store: &S) {
    let key = fresh_key("tighten");
    // One permit per hour is booked; a caller then requires one per day.
    granted(
        store,
        &key,
        &hourly(1),
        ReserveRequest::new(1, Duration::ZERO),
    )
    .await;
    let daily = Rate::new(NonZeroU32::MIN, 24 * HOUR)
        .unwrap_or_else(|error| panic!("test rate is valid: {error}"));
    let retry_after =
        match reserve(store, &key, &daily, ReserveRequest::new(1, Duration::ZERO)).await {
            Err(Denied::Later { retry_after }) => retry_after,
            other => panic!("expected a refusal with a retry_after, got {other:?}"),
        };
    assert_about(
        retry_after,
        24 * HOUR,
        "the hour booked counts as a day's interval under the stricter rate",
    );
}

/// Concurrent callers never get more than the burst between them.
pub async fn concurrent_callers_never_exceed_the_burst<S: LimitStore + 'static>(store: Arc<S>) {
    let key = fresh_key("concurrent");
    let rate = hourly(5);
    // Every caller is spawned before any is awaited, so they contend.
    let mut callers = Vec::with_capacity(32);
    for _ in 0..32 {
        let (store, key) = (Arc::clone(&store), key.clone());
        callers.push(tokio::spawn(async move {
            reserve(&*store, &key, &rate, ReserveRequest::new(1, Duration::ZERO))
                .await
                .is_ok()
        }));
    }
    let mut admitted = 0;
    for caller in callers {
        if caller
            .await
            .unwrap_or_else(|error| panic!("caller task: {error}"))
        {
            admitted += 1;
        }
    }
    assert_eq!(
        admitted, 5,
        "exactly the burst is admitted under contention"
    );
}

/// Runs every case against `store`.
pub async fn run_all<S: LimitStore + 'static>(store: Arc<S>) {
    burst_then_refusal_consumes_nothing(&*store).await;
    keys_are_isolated(&*store).await;
    permits_beyond_the_burst_are_never_granted(&*store).await;
    penalty_blocks_an_idle_key_up_to_the_cap(&*store).await;
    cancel_refunds_only_the_tail_once(&*store).await;
    repeated_reservation_id_returns_the_original_grant(&*store).await;
    a_busy_key_enforces_the_stricter_rate(&*store).await;
    tightening_a_busy_key_rebases_its_schedule(&*store).await;
    concurrent_callers_never_exceed_the_burst(store).await;
}
