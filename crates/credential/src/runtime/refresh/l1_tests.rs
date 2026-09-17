use std::time::Duration;

use super::*;

#[test]
fn circuit_breaker_tracking_is_bounded() {
    let coord = L1RefreshCoalescer::new();
    for i in 0..6000 {
        coord.record_failure(&format!("cred-{i}"));
    }
    assert!(
        coord.circuit_breaker_len_for_test() <= MAX_TRACKED_CIRCUIT_BREAKERS.get(),
        "LRU cap should prevent unbounded growth (issue #278)"
    );
}

#[tokio::test]
async fn first_caller_wins() {
    let coord = L1RefreshCoalescer::new();
    let attempt = coord.try_refresh("cred-1");
    assert!(matches!(attempt, RefreshAttempt::Winner));
    assert_eq!(coord.in_flight_count(), 1);
}

#[tokio::test]
async fn second_caller_waits() {
    let coord = L1RefreshCoalescer::new();
    let first = coord.try_refresh("cred-1");
    assert!(matches!(first, RefreshAttempt::Winner));

    let second = coord.try_refresh("cred-1");
    assert!(matches!(second, RefreshAttempt::Waiter(_)));
}

#[tokio::test]
async fn complete_removes_in_flight_entry() {
    let coord = L1RefreshCoalescer::new();
    let _ = coord.try_refresh("cred-1");
    assert_eq!(coord.in_flight_count(), 1);

    coord.complete("cred-1", L1Completion::StateAdvanced);
    assert_eq!(coord.in_flight_count(), 0);
}

#[tokio::test]
async fn complete_wakes_waiters() {
    let coord = Arc::new(L1RefreshCoalescer::new());

    assert!(matches!(
        coord.try_refresh("cred-1"),
        RefreshAttempt::Winner
    ));

    let waiter_rx = match coord.try_refresh("cred-1") {
        RefreshAttempt::Waiter(rx) => rx,
        RefreshAttempt::Winner => panic!("expected Waiter"),
    };

    let waiter = tokio::spawn(waiter_rx);

    coord.complete("cred-1", L1Completion::RetryUnsafe);

    let result = waiter
        .await
        .expect("waiter task must join")
        .expect("winner must send a completion policy");
    assert_eq!(result, L1Completion::RetryUnsafe);
}

#[tokio::test]
async fn independent_credentials_do_not_interfere() {
    let coord = L1RefreshCoalescer::new();

    let a = coord.try_refresh("cred-a");
    let b = coord.try_refresh("cred-b");

    assert!(matches!(a, RefreshAttempt::Winner));
    assert!(matches!(b, RefreshAttempt::Winner));
    assert_eq!(coord.in_flight_count(), 2);

    coord.complete("cred-a", L1Completion::StateAdvanced);
    assert_eq!(coord.in_flight_count(), 1);

    coord.complete("cred-b", L1Completion::StateAdvanced);
    assert_eq!(coord.in_flight_count(), 0);
}

#[tokio::test]
async fn complete_on_unknown_credential_is_noop() {
    let coord = L1RefreshCoalescer::new();
    coord.complete("nonexistent", L1Completion::NoStateChange);
    assert_eq!(coord.in_flight_count(), 0);
}

#[tokio::test]
async fn winner_after_complete_can_refresh_again() {
    let coord = L1RefreshCoalescer::new();

    let first = coord.try_refresh("cred-1");
    assert!(matches!(first, RefreshAttempt::Winner));
    coord.complete("cred-1", L1Completion::StateAdvanced);

    let second = coord.try_refresh("cred-1");
    assert!(matches!(second, RefreshAttempt::Winner));
    coord.complete("cred-1", L1Completion::StateAdvanced);
}

#[tokio::test]
async fn multiple_waiters_all_notified() {
    let coord = Arc::new(L1RefreshCoalescer::new());

    assert!(matches!(
        coord.try_refresh("cred-1"),
        RefreshAttempt::Winner
    ));

    let mut handles = Vec::new();
    for _ in 0..5 {
        let rx = match coord.try_refresh("cred-1") {
            RefreshAttempt::Waiter(rx) => rx,
            RefreshAttempt::Winner => panic!("expected Waiter"),
        };
        handles.push(tokio::spawn(rx));
    }

    coord.complete("cred-1", L1Completion::NoStateChange);

    for handle in handles {
        assert_eq!(
            handle
                .await
                .expect("waiter task must join")
                .expect("winner must send a completion policy"),
            L1Completion::NoStateChange
        );
    }
}

/// Regression for GitHub issue #268: pushing the waiter's sender into
/// the in-flight entry happens under the same map mutex the winner
/// acquires in `complete()`, so the winner cannot race past a waiter's
/// registration. Exercises the tight timing that used to lose wakeups
/// under `Notify::notify_waiters()` (where the waiter had to
/// `enable()` its `Notified` future after the lock was released).
#[tokio::test]
async fn waiter_registered_under_lock_is_never_missed() {
    let coord = Arc::new(L1RefreshCoalescer::new());

    assert!(matches!(
        coord.try_refresh("cred-1"),
        RefreshAttempt::Winner
    ));

    let rx = match coord.try_refresh("cred-1") {
        RefreshAttempt::Waiter(rx) => rx,
        RefreshAttempt::Winner => panic!("expected Waiter"),
    };
    coord.complete("cred-1", L1Completion::StateAdvanced);

    tokio::time::timeout(Duration::from_millis(50), rx)
        .await
        .expect("waiter must observe completion even when it fires immediately")
        .expect("sender must be fired, not dropped");
}

#[tokio::test]
async fn default_creates_empty_coalescer() {
    let coord = L1RefreshCoalescer::default();
    assert_eq!(coord.in_flight_count(), 0);
}

#[tokio::test]
async fn circuit_breaker_opens_after_max_failures() {
    let coord = L1RefreshCoalescer::new();
    for _ in 0..5 {
        coord.record_failure("cred-1");
    }
    assert!(coord.is_circuit_open("cred-1"));
}

#[tokio::test]
async fn circuit_breaker_closed_initially() {
    let coord = L1RefreshCoalescer::new();
    assert!(!coord.is_circuit_open("cred-1"));
}

#[tokio::test]
async fn circuit_breaker_resets_on_success() {
    let coord = L1RefreshCoalescer::new();
    for _ in 0..5 {
        coord.record_failure("cred-1");
    }
    assert!(coord.is_circuit_open("cred-1"));
    coord.record_success("cred-1");
    assert!(!coord.is_circuit_open("cred-1"));
}

#[tokio::test]
async fn circuit_breaker_below_threshold_stays_closed() {
    let coord = L1RefreshCoalescer::new();
    for _ in 0..4 {
        coord.record_failure("cred-1");
    }
    assert!(!coord.is_circuit_open("cred-1"));
}

/// B8: Verifies that dropping a Winner without calling complete()
/// leaves the in-flight entry, and that a subsequent sync complete()
/// (as scopeguard would call) cleans it up so new callers become Winners.
#[tokio::test]
async fn inflight_entry_cleaned_after_sync_complete() {
    let coord = L1RefreshCoalescer::new();

    let attempt = coord.try_refresh("cred-1");
    assert!(matches!(attempt, RefreshAttempt::Winner));
    assert_eq!(coord.in_flight_count(), 1);

    drop(attempt);
    assert_eq!(coord.in_flight_count(), 1);

    coord.complete("cred-1", L1Completion::NoStateChange);
    assert_eq!(coord.in_flight_count(), 0);

    let attempt2 = coord.try_refresh("cred-1");
    assert!(matches!(attempt2, RefreshAttempt::Winner));
    coord.complete("cred-1", L1Completion::StateAdvanced);
}

#[tokio::test]
async fn default_coalescer_has_default_permits() {
    let coord = L1RefreshCoalescer::new();
    assert_eq!(coord.available_permits(), DEFAULT_MAX_CONCURRENT_REFRESHES);
}

/// Regression guard: a waiter that drops its receiver (cancellation)
/// must not prevent the winner from completing.
#[tokio::test]
async fn cancelled_waiter_does_not_stall_winner() {
    let coord = Arc::new(L1RefreshCoalescer::new());
    assert!(matches!(
        coord.try_refresh("cred-1"),
        RefreshAttempt::Winner
    ));

    match coord.try_refresh("cred-1") {
        RefreshAttempt::Waiter(rx) => drop(rx),
        RefreshAttempt::Winner => panic!("expected Waiter"),
    }

    coord.complete("cred-1", L1Completion::NoStateChange);
    assert_eq!(coord.in_flight_count(), 0);
}

#[tokio::test]
async fn closed_waiters_are_pruned_without_completing_winner() {
    let coord = L1RefreshCoalescer::new();
    assert!(matches!(
        coord.try_refresh("cred-1"),
        RefreshAttempt::Winner
    ));
    let receiver = match coord.try_refresh("cred-1") {
        RefreshAttempt::Waiter(receiver) => receiver,
        RefreshAttempt::Winner => panic!("expected waiter"),
    };
    assert_eq!(coord.waiter_count_for_test("cred-1"), 1);

    drop(receiver);
    coord.prune_closed_waiters("cred-1");

    assert_eq!(coord.waiter_count_for_test("cred-1"), 0);
    assert_eq!(
        coord.in_flight_count(),
        1,
        "only the winner's owned lease may complete the in-flight entry"
    );
    coord.complete("cred-1", L1Completion::NoStateChange);
}
