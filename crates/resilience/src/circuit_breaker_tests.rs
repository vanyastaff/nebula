use std::time::Duration;

use super::*;
use crate::{
    CallError, PolicyContext, RecordingSink,
    cancellation::CancellationContext,
    classifier::{ErrorClass, FnClassifier},
    sink::CircuitState as CS,
};

fn default_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_threshold: 3,
        reset_timeout: Duration::from_millis(100),
        max_half_open_operations: 1,
        half_open_success_threshold: None,
        min_operations: 1,
        count_timeouts_as_failures: true,
        break_duration_multiplier: 1.0,
        max_break_duration: Duration::from_mins(5),
        slow_call_threshold: None,
        slow_call_rate_threshold: 1.0,
    }
}

#[tokio::test]
async fn opens_after_failure_threshold() {
    let cb = CircuitBreaker::new(default_config()).unwrap();
    for _ in 0..3 {
        let _ = cb
            .call::<(), _, _>(|| Box::pin(async { Err("fail") }))
            .await;
    }
    let err: CallError<&str> = cb
        .call::<(), _, _>(|| Box::pin(async { Ok(()) }))
        .await
        .unwrap_err();
    assert!(matches!(err, CallError::CircuitOpen));
}

#[tokio::test]
async fn cancelled_does_not_trip_breaker() {
    let cb = CircuitBreaker::new(default_config()).unwrap();
    for _ in 0..10 {
        cb.record_outcome(Outcome::Cancelled);
    }
    let result = cb.call::<u32, &str, _>(|| Box::pin(async { Ok(42) })).await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn policy_context_cancellation_does_not_trip_breaker() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        ..default_config()
    })
    .unwrap();
    let cancellation = CancellationContext::with_reason("shutdown");
    let context = PolicyContext::from_cancellation(cancellation.clone());
    cancellation.cancel();

    let result = cb
        .call_with_policy_context::<(), &str, _>(&context, || {
            Box::pin(async { Ok::<(), &str>(()) })
        })
        .await;

    assert!(matches!(result, Err(CallError::Cancelled { .. })));
    assert_eq!(cb.circuit_state(), CS::Closed);
    assert_eq!(cb.stats().total, 0);
}

#[tokio::test]
async fn policy_context_deadline_records_timeout_outcome() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        min_operations: 1,
        count_timeouts_as_failures: true,
        ..default_config()
    })
    .unwrap();
    let context = PolicyContext::with_timeout(Duration::from_millis(1));

    let result = cb
        .call_with_policy_context::<(), &str, _>(&context, || {
            Box::pin(async {
                tokio::time::sleep(Duration::from_mins(1)).await;
                Ok::<(), &str>(())
            })
        })
        .await;

    assert!(matches!(result, Err(CallError::Timeout(_))));
    assert_eq!(cb.circuit_state(), CS::Open);
}

#[tokio::test]
async fn classified_timeout_respects_timeout_counting_config() {
    #[derive(Debug)]
    struct TimeoutErr;

    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        min_operations: 1,
        count_timeouts_as_failures: false,
        ..default_config()
    })
    .unwrap();
    let classifier = FnClassifier::new(|_: &TimeoutErr| ErrorClass::Timeout);

    let result = cb
        .call_with_classifier(&classifier, || {
            Box::pin(async { Err::<(), TimeoutErr>(TimeoutErr) })
        })
        .await;

    assert!(matches!(result, Err(CallError::Operation(_))));
    assert_eq!(cb.circuit_state(), CS::Closed);
    assert_eq!(cb.stats().total, 0);
}

#[tokio::test]
async fn emits_state_change_event_on_open() {
    let sink = RecordingSink::new();
    let cb = CircuitBreaker::new(default_config())
        .unwrap()
        .with_sink(sink.clone());
    for _ in 0..3 {
        let _ = cb
            .call::<(), &str, _>(|| Box::pin(async { Err("fail") }))
            .await;
    }
    assert!(sink.has_state_change(CS::Open));
}

#[tokio::test]
async fn config_error_on_zero_threshold() {
    let result = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 0,
        ..default_config()
    });
    assert!(result.is_err());
}

#[tokio::test]
async fn half_open_enforces_max_probes() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        max_half_open_operations: 1,
        ..default_config()
    })
    .unwrap();

    // Trip the breaker
    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }
    assert_eq!(cb.circuit_state(), CS::Open);

    // Wait for reset timeout
    tokio::time::sleep(Duration::from_millis(110)).await;

    // First probe should succeed (transitions to HalfOpen)
    assert!(cb.try_acquire::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    // Second probe should be rejected (max_probes=1 reached)
    assert!(matches!(
        cb.try_acquire::<&str>(),
        Err(CallError::CircuitOpen)
    ));

    // After the probe succeeds, breaker closes and allows new calls
    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::Closed);
    assert!(cb.try_acquire::<&str>().is_ok());
}

#[tokio::test]
async fn half_open_requires_success_threshold_before_closing() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        max_half_open_operations: 2,
        ..default_config()
    })
    .unwrap();

    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }
    assert_eq!(cb.circuit_state(), CS::Open);

    tokio::time::sleep(Duration::from_millis(110)).await;

    assert!(cb.try_acquire::<&str>().is_ok());
    assert!(cb.try_acquire::<&str>().is_ok());
    assert!(matches!(
        cb.try_acquire::<&str>(),
        Err(CallError::CircuitOpen)
    ));

    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::Closed);
}

#[tokio::test]
async fn ignored_timeout_in_half_open_releases_probe_without_reopening() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        count_timeouts_as_failures: false,
        ..default_config()
    })
    .unwrap();

    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }
    assert_eq!(cb.circuit_state(), CS::Open);

    tokio::time::sleep(Duration::from_millis(110)).await;

    assert!(cb.try_acquire::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    cb.record_outcome(Outcome::Timeout);
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    assert!(cb.try_acquire::<&str>().is_ok());
    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::Closed);
}

#[tokio::test]
async fn half_open_failure_reopens_breaker() {
    let sink = RecordingSink::new();
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        max_half_open_operations: 1,
        ..default_config()
    })
    .unwrap()
    .with_sink(sink.clone());

    // Trip the breaker
    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }

    // Wait for reset timeout
    tokio::time::sleep(Duration::from_millis(110)).await;

    // Enter HalfOpen
    assert!(cb.try_acquire::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    // Probe fails → back to Open
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);
}

#[tokio::test]
async fn dropped_call_releases_probe_slot() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            max_half_open_operations: 1,
            ..default_config()
        })
        .unwrap(),
    );

    // Trip the breaker
    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }

    // Wait for reset timeout
    tokio::time::sleep(Duration::from_millis(110)).await;

    // Start a call that will be dropped mid-operation
    let cb2 = Arc::clone(&cb);
    tokio::select! {
        _ = cb2.call(|| Box::pin(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<(), &str>(())
        })) => unreachable!(),
        () = tokio::time::sleep(Duration::from_millis(5)) => {
            // Future dropped — probe guard should release the slot
        }
    }

    // The probe slot should be freed. Wait for reset again and try a new probe.
    // Since the cancelled probe decremented half_open_probes, the next
    // Open→HalfOpen transition should work.
    tokio::time::sleep(Duration::from_millis(110)).await;

    // This must succeed — the probe slot was properly released
    assert!(cb.try_acquire::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    // Complete the probe successfully
    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::Closed);
}

#[tokio::test]
async fn force_open_rejects_calls() {
    let cb = CircuitBreaker::new(default_config()).unwrap();
    cb.force_open();
    assert_eq!(cb.circuit_state(), CS::Open);
    let err: CallError<&str> = cb
        .call::<(), _, _>(|| Box::pin(async { Ok(()) }))
        .await
        .unwrap_err();
    assert!(matches!(err, CallError::CircuitOpen));
}

#[tokio::test]
async fn force_close_resets_circuit() {
    let cb = CircuitBreaker::new(default_config()).unwrap();
    for _ in 0..3 {
        cb.record_outcome(Outcome::Failure);
    }
    assert_eq!(cb.circuit_state(), CS::Open);
    cb.force_close();
    assert_eq!(cb.circuit_state(), CS::Closed);
    let result = cb.call::<u32, &str, _>(|| Box::pin(async { Ok(42) })).await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn on_state_change_fires_on_open() {
    let transitions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let t = transitions.clone();

    let cb = CircuitBreaker::new(default_config())
        .unwrap()
        .on_state_change(move |from, to| {
            t.lock().unwrap().push((from, to));
        });

    for _ in 0..3 {
        let _ = cb
            .call::<(), &str, _>(|| Box::pin(async { Err("fail") }))
            .await;
    }

    let t = transitions.lock().unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0], (CS::Closed, CS::Open));
    drop(t);
}

#[tokio::test]
async fn dynamic_break_duration_increases_on_repeated_opens() {
    use crate::clock::MockClock;
    let clock = Arc::new(MockClock::new());
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 2,
        reset_timeout: Duration::from_millis(100),
        max_half_open_operations: 1,
        half_open_success_threshold: None,
        min_operations: 1,
        count_timeouts_as_failures: true,
        break_duration_multiplier: 2.0,
        max_break_duration: Duration::from_secs(10),
        slow_call_threshold: None,
        slow_call_rate_threshold: 1.0,
    })
    .unwrap()
    .with_clock(Arc::clone(&clock) as Arc<dyn Clock>);

    // First trip
    cb.record_outcome(Outcome::Failure);
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);

    // Wait 110ms (> first reset_timeout of 100ms)
    clock.advance(Duration::from_millis(110));
    assert!(cb.try_acquire::<&str>().is_ok());
    assert_eq!(cb.circuit_state(), CS::HalfOpen);

    // Fail again → consecutive_opens = 2, effective timeout = 200ms
    cb.record_outcome(Outcome::Failure);
    assert_eq!(cb.circuit_state(), CS::Open);

    // Wait 110ms — NOT enough (need 200ms)
    clock.advance(Duration::from_millis(110));
    assert!(matches!(
        cb.try_acquire::<&str>(),
        Err(CallError::CircuitOpen)
    ));

    // Wait 100ms more (total 220ms > 200ms)
    clock.advance(Duration::from_millis(100));
    assert!(cb.try_acquire::<&str>().is_ok());
}

#[tokio::test]
async fn slow_calls_trip_breaker() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 100,
        slow_call_threshold: Some(Duration::from_millis(10)),
        slow_call_rate_threshold: 0.5,
        min_operations: 3,
        ..default_config()
    })
    .unwrap();

    // 3 slow successes -> 100% slow rate > 50% threshold
    cb.record_outcome(Outcome::SlowSuccess);
    cb.record_outcome(Outcome::SlowSuccess);
    cb.record_outcome(Outcome::SlowSuccess);
    assert_eq!(cb.circuit_state(), CS::Open);
}

#[test]
fn classify_outcome_detects_slow_calls() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        slow_call_threshold: Some(Duration::from_millis(100)),
        ..CircuitBreakerConfig::default()
    })
    .unwrap();

    assert!(matches!(
        cb.classify_outcome(true, Duration::from_millis(50)),
        Outcome::Success
    ));
    assert!(matches!(
        cb.classify_outcome(true, Duration::from_millis(150)),
        Outcome::SlowSuccess
    ));
    assert!(matches!(
        cb.classify_outcome(false, Duration::from_millis(150)),
        Outcome::SlowFailure
    ));
    assert!(matches!(
        cb.classify_outcome(false, Duration::from_millis(50)),
        Outcome::Failure
    ));
}

#[tokio::test]
async fn slow_calls_below_threshold_dont_trip() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 100,
        slow_call_threshold: Some(Duration::from_millis(10)),
        slow_call_rate_threshold: 0.5,
        min_operations: 4,
        ..default_config()
    })
    .unwrap();

    // 1 slow + 3 normal = 25% < 50%
    cb.record_outcome(Outcome::SlowSuccess);
    cb.record_outcome(Outcome::Success);
    cb.record_outcome(Outcome::Success);
    cb.record_outcome(Outcome::Success);
    assert_eq!(cb.circuit_state(), CS::Closed);
}

/// Pins constructor validity of the `Default` impl — the config the L1
/// refresh fallback in `crates/credential` reaches for when its static
/// config is rejected. If `CircuitBreakerConfig::default()` stopped
/// satisfying `validate()`, that fallback would hit its nested
/// `unwrap_or_else`'s `unreachable!` (l1.rs) instead of degrading
/// gracefully, so this pins the invariant the fallback relies on.
#[test]
fn default_config_is_constructor_valid() {
    assert!(
        CircuitBreakerConfig::default().validate().is_ok(),
        "the Default impl must produce a config that passes validate()"
    );
    assert!(
        CircuitBreaker::new(CircuitBreakerConfig::default()).is_ok(),
        "the Default impl config must be accepted by CircuitBreaker::new"
    );
}

// ── C1: min_operations validation ────────────────────────────────────

#[test]
fn rejects_min_operations_zero() {
    let config = CircuitBreakerConfig {
        min_operations: 0,
        ..default_config()
    };
    let err = CircuitBreaker::new(config).unwrap_err();
    assert_eq!(err.field, "min_operations");
}
