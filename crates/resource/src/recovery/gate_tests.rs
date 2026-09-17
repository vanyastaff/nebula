use super::*;

fn default_gate() -> RecoveryGate {
    RecoveryGate::new(RecoveryGateConfig::default())
}

#[test]
fn idle_gate_grants_ticket() {
    let gate = default_gate();
    let ticket = gate.try_begin().expect("should grant ticket");
    assert_eq!(ticket.attempt(), 1);
    ticket.resolve();
}

#[test]
fn second_caller_gets_waiter() {
    let gate = default_gate();
    let _ticket = gate.try_begin().expect("first caller wins");
    match gate.try_begin() {
        Err(TryBeginError::AlreadyInProgress(_)) => {}, // expected
        other => panic!("expected AlreadyInProgress, got: {other:?}"),
    }
}

#[test]
fn resolve_returns_to_idle() {
    let gate = default_gate();
    let ticket = gate.try_begin().unwrap();
    ticket.resolve();
    assert!(matches!(gate.state(), GateState::Idle));
}

#[test]
fn fail_transient_sets_failed_state() {
    let gate = default_gate();
    let ticket = gate.try_begin().unwrap();
    ticket.fail_transient("connection refused");
    match gate.state() {
        GateState::Failed {
            message, attempt, ..
        } => {
            assert_eq!(message, "connection refused");
            assert_eq!(attempt, 1);
        },
        other => panic!("expected Failed, got: {other:?}"),
    }
}

#[test]
fn fail_permanent_blocks_further_attempts() {
    let gate = default_gate();
    let ticket = gate.try_begin().unwrap();
    ticket.fail_permanent("certificate expired");
    match gate.try_begin() {
        Err(TryBeginError::PermanentlyFailed { message }) => {
            assert_eq!(message, "certificate expired");
        },
        other => panic!("expected PermanentlyFailed, got: {other:?}"),
    }
}

#[test]
fn drop_without_resolve_auto_fails() {
    let gate = default_gate();
    {
        let _ticket = gate.try_begin().unwrap();
        // Dropped here without resolve/fail.
    }
    assert!(matches!(gate.state(), GateState::Failed { .. }));
}

#[test]
fn reset_clears_permanent_failure() {
    let gate = default_gate();
    let ticket = gate.try_begin().unwrap();
    ticket.fail_permanent("dead");
    assert!(matches!(gate.state(), GateState::PermanentlyFailed { .. }));
    gate.reset();
    assert!(matches!(gate.state(), GateState::Idle));
}

#[test]
fn backoff_escalates_exponentially() {
    let base = Duration::from_millis(100);
    assert_eq!(compute_backoff(base, 1), Duration::from_millis(100));
    assert_eq!(compute_backoff(base, 2), Duration::from_millis(200));
    assert_eq!(compute_backoff(base, 3), Duration::from_millis(400));
    assert_eq!(compute_backoff(base, 4), Duration::from_millis(800));
}

#[test]
fn backoff_caps_at_five_minutes() {
    let base = Duration::from_mins(1);
    // 60 * 2^4 = 960s > 300s cap
    assert_eq!(compute_backoff(base, 5), MAX_BACKOFF);
}

#[test]
fn equal_jitter_stays_within_half_to_nominal() {
    let nominal = Duration::from_secs(10);
    for _ in 0..1_000 {
        let jittered = crate::jitter::apply_jitter(nominal, EQUAL_JITTER_SPREAD);
        assert!(
            jittered >= nominal / 2 && jittered <= nominal,
            "equal jitter must spread over [nominal/2, nominal], got {jittered:?}"
        );
    }
}

#[test]
fn equal_jitter_zero_backoff_stays_zero() {
    // Zero-backoff test configs rely on retry_at being immediately
    // expired — jitter must not resurrect a delay from nothing.
    assert_eq!(
        crate::jitter::apply_jitter(Duration::ZERO, EQUAL_JITTER_SPREAD),
        Duration::ZERO
    );
    // Sub-2ns backoff has a zero half; passes through unjittered.
    assert_eq!(
        crate::jitter::apply_jitter(Duration::from_nanos(1), EQUAL_JITTER_SPREAD),
        Duration::from_nanos(1)
    );
}

#[test]
fn failed_retry_at_lands_within_the_jitter_window() {
    let gate = RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: Duration::from_secs(8),
    });
    let before = Instant::now();
    let ticket = gate.try_begin().expect("gate starts idle");
    ticket.fail_transient("probe refused");
    match gate.state() {
        GateState::Failed { retry_at, .. } => {
            let delay = retry_at.duration_since(before);
            assert!(
                delay >= Duration::from_secs(4) && delay <= Duration::from_secs(8),
                "attempt 1 delay must land in [nominal/2, nominal] = [4s, 8s], got {delay:?}"
            );
        },
        other => panic!("expected Failed, got: {other:?}"),
    }
}

#[test]
fn retry_after_expired_allows_new_attempt() {
    let config = RecoveryGateConfig {
        max_attempts: 5,
        base_backoff: Duration::from_millis(0), // zero backoff for test
    };
    let gate = RecoveryGate::new(config);

    let ticket = gate.try_begin().unwrap();
    ticket.fail_transient("timeout");

    // Backoff is 0ms, so retry_at is already in the past.
    let ticket2 = gate.try_begin().expect("should allow retry after expiry");
    assert_eq!(ticket2.attempt(), 2);
    ticket2.resolve();
}

#[test]
fn max_attempts_triggers_permanent_failure() {
    let config = RecoveryGateConfig {
        max_attempts: 2,
        base_backoff: Duration::from_millis(0),
    };
    let gate = RecoveryGate::new(config);

    // Attempt 1
    let t1 = gate.try_begin().unwrap();
    t1.fail_transient("fail 1");

    // Attempt 2
    let t2 = gate.try_begin().unwrap();
    t2.fail_transient("fail 2");

    // Attempt 3 exceeds max_attempts=2 → permanent
    match gate.try_begin() {
        Err(TryBeginError::PermanentlyFailed { message }) => {
            assert!(message.contains("exceeded"), "msg: {message}");
        },
        other => panic!("expected PermanentlyFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn waiter_unblocks_on_resolve() {
    let gate = default_gate();
    let ticket = gate.try_begin().unwrap();

    let gate2 = gate.clone();
    let handle = tokio::spawn(async move {
        match gate2.try_begin() {
            Err(TryBeginError::AlreadyInProgress(waiter)) => waiter.wait().await,
            other => panic!("expected AlreadyInProgress, got: {other:?}"),
        }
    });

    // Give the spawned task a moment to reach the wait point.
    tokio::task::yield_now().await;
    ticket.resolve();

    let state = handle.await.unwrap();
    assert!(matches!(state, GateState::Idle));
}

#[tokio::test]
async fn concurrent_try_begin_only_one_wins() {
    let gate = default_gate();
    let mut winners = 0u32;
    let mut waiters = 0u32;

    for _ in 0..10 {
        match gate.try_begin() {
            Ok(ticket) => {
                winners += 1;
                ticket.resolve();
            },
            Err(TryBeginError::AlreadyInProgress(_)) => {
                waiters += 1;
            },
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }

    // All calls are sequential in this test, so each one should win
    // after the previous resolves.
    assert_eq!(winners, 10);
    assert_eq!(waiters, 0);
}

#[test]
fn fail_transient_then_resolve_cycle() {
    let config = RecoveryGateConfig {
        max_attempts: 10,
        base_backoff: Duration::from_millis(0),
    };
    let gate = RecoveryGate::new(config);

    // Fail twice, then succeed.
    let t = gate.try_begin().unwrap();
    t.fail_transient("fail 1");

    let t = gate.try_begin().unwrap();
    t.fail_transient("fail 2");

    let t = gate.try_begin().unwrap();
    assert_eq!(t.attempt(), 3);
    t.resolve();

    assert!(matches!(gate.state(), GateState::Idle));
}

/// Correctness-pin (NOT a RED-then-fix; no `enable()` change).
///
/// `RecoveryWaiter::wait` creates the `Notified` future *before* loading
/// the gate state, and every gate notifier uses `notify_waiters()` (not
/// `notify_one()`). tokio's documented contract: `notified()` captures
/// the `notify_waiters` call-count at creation, so a `notify_waiters()`
/// firing between the future's creation and its first `.await` is
/// delivered on first poll **without** `enable()` and without a prior
/// poll. This test pins that exact property on a bare `Notify` mirroring
/// `wait`'s construction order, so a future refactor that switches the
/// gate to `notify_one()` (or reorders the create-then-load) would break
/// this pin and surface the regression.
#[tokio::test]
async fn notify_waiters_between_notified_creation_and_await_is_delivered() {
    let notify = Notify::new();

    // Mirror `RecoveryWaiter::wait`: create the future first…
    let fut = notify.notified();

    // …then fire `notify_waiters()` *before* the future is ever polled
    // / awaited (no `enable()` call). Per the tokio contract this is
    // captured by the count taken at `notified()` creation.
    notify.notify_waiters();

    // The await must complete immediately on first poll — a missed
    // wakeup would hang here. A bounded timeout converts a contract
    // regression into a fast failure instead of a hung test.
    tokio::time::timeout(Duration::from_secs(1), fut)
        .await
        .expect(
            "notify_waiters() fired after notified() creation but before \
             .await must be delivered without enable() (tokio contract \
             — this is why RecoveryWaiter::wait's ordering is correct)",
        );
}

/// End-to-end companion: a `RecoveryWaiter` obtained while a ticket is
/// held must unblock once the ticket resolves, even when `resolve()`
/// races immediately after the waiter is constructed (the
/// create-notified-before-state-load ordering is what guarantees no lost
/// wakeup). Deterministic — no sleeps, no yield-budget guessing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_waiter_unblocks_when_resolve_races_the_wait() {
    let gate = default_gate();
    let ticket = gate.try_begin().expect("first caller wins the ticket");

    let waiter = match gate.try_begin() {
        Err(TryBeginError::AlreadyInProgress(w)) => w,
        other => panic!("expected AlreadyInProgress, got: {other:?}"),
    };

    // Resolve and wait are started "simultaneously": even if `resolve()`
    // (state→Idle + notify_waiters) lands between the waiter's
    // `notified()` creation and its `.await`, the wait must still
    // observe a non-`InProgress` state or receive the captured notify —
    // never hang.
    let wait_task = tokio::spawn(async move { waiter.wait().await });
    ticket.resolve();

    let state = tokio::time::timeout(Duration::from_secs(2), wait_task)
        .await
        .expect("waiter must not hang when resolve races the wait")
        .expect("wait task must not panic");
    assert!(
        matches!(state, GateState::Idle),
        "resolve returns the gate to Idle; the waiter must observe it"
    );
}
