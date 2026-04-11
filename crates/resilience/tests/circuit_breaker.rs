//! Integration tests for CircuitBreaker — full lifecycle through call().

use std::{sync::Arc, time::Duration};

use nebula_resilience::{
    CallError,
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
    sink::CircuitState,
};

fn test_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_threshold: 3,
        reset_timeout: Duration::from_millis(50),
        max_half_open_operations: 1,
        min_operations: 1,
        ..Default::default()
    }
}

// ── Full lifecycle: Closed → Open → HalfOpen → Closed ──────────────────────

#[tokio::test]
async fn full_lifecycle_through_call() {
    let cb = Arc::new(CircuitBreaker::new(test_config()).unwrap());

    // Phase 1: Closed — operations succeed
    for _ in 0..3 {
        let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(42) })).await;
        assert_eq!(result.unwrap(), 42);
    }
    assert_eq!(cb.circuit_state(), CircuitState::Closed);

    // Phase 2: Failures trip the breaker → Open
    for _ in 0..3 {
        let _ = cb
            .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
            .await;
    }
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // Phase 3: Calls rejected while Open
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(42) })).await;
    assert!(matches!(result, Err(CallError::CircuitOpen)));

    // Phase 4: Wait for reset_timeout → HalfOpen
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Phase 5: Probe succeeds → Closed
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(99) })).await;
    assert_eq!(result.unwrap(), 99);
    assert_eq!(cb.circuit_state(), CircuitState::Closed);

    // Phase 6: Normal operations resume
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(1) })).await;
    assert_eq!(result.unwrap(), 1);
}

// ── HalfOpen probe failure → back to Open ───────────────────────────────────

#[tokio::test]
async fn half_open_probe_failure_reopens() {
    let cb = Arc::new(CircuitBreaker::new(test_config()).unwrap());

    // Trip the breaker
    for _ in 0..3 {
        let _ = cb
            .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
            .await;
    }
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // Wait for HalfOpen
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Probe fails → back to Open
    let result = cb
        .call(|| Box::pin(async { Err::<u32, &str>("probe fail") }))
        .await;
    assert!(matches!(result, Err(CallError::Operation("probe fail"))));
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // Still rejected
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(42) })).await;
    assert!(matches!(result, Err(CallError::CircuitOpen)));
}

// ── Dynamic break duration escalates ────────────────────────────────────────

#[tokio::test]
async fn dynamic_break_duration_escalates_with_consecutive_opens() {
    let cb = Arc::new(
        CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(30),
            max_half_open_operations: 1,
            min_operations: 1,
            break_duration_multiplier: 2.0,
            max_break_duration: Duration::from_secs(10),
            ..Default::default()
        })
        .unwrap(),
    );

    // First trip: reset_timeout = 30ms
    let _ = cb
        .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
        .await;
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    tokio::time::sleep(Duration::from_millis(40)).await;

    // Probe fails → second trip: effective timeout = 60ms
    let _ = cb
        .call(|| Box::pin(async { Err::<u32, &str>("fail again") }))
        .await;
    assert_eq!(cb.circuit_state(), CircuitState::Open);

    // 40ms is NOT enough (need 60ms)
    tokio::time::sleep(Duration::from_millis(40)).await;
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(42) })).await;
    assert!(
        matches!(result, Err(CallError::CircuitOpen)),
        "should still be open after 40ms (need 60ms)"
    );

    // Wait remaining 30ms (total ~70ms > 60ms)
    tokio::time::sleep(Duration::from_millis(30)).await;
    let result = cb.call(|| Box::pin(async { Ok::<u32, &str>(42) })).await;
    assert_eq!(result.unwrap(), 42);
    assert_eq!(cb.circuit_state(), CircuitState::Closed);
}
