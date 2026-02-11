//! Integration tests for pattern composition
//!
//! Tests combinations of multiple resilience patterns:
//! - Retry + Circuit Breaker
//! - Timeout + Bulkhead
//! - Circuit Breaker + Timeout + Retry
//! - Full policy composition

use nebula_resilience::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::time::sleep;

/// Test: Circuit breaker opens after threshold failures
#[tokio::test]
async fn test_circuit_breaker_opens_after_failures() {
    // Circuit breaker with 3 failure threshold, 2 second reset
    // Set min_operations to 1 so circuit opens as soon as failure threshold is reached
    let config = CircuitBreakerConfig::<3, 2000>::new().with_min_operations(1);
    let circuit_breaker = Arc::new(CircuitBreaker::new(config).unwrap());

    let attempt_count = Arc::new(AtomicU32::new(0));

    // Cause 3 failures to open circuit
    for _ in 0..3 {
        let attempt_count_clone = Arc::clone(&attempt_count);
        let result = circuit_breaker
            .execute(|| {
                let attempt_count = Arc::clone(&attempt_count_clone);
                async move {
                    attempt_count.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(ResilienceError::custom("Failure"))
                }
            })
            .await;

        assert!(result.is_err());
    }

    // Circuit should be open now
    let stats = circuit_breaker.stats().await;
    assert_eq!(
        stats.state,
        nebula_resilience::patterns::circuit_breaker::State::Open
    );

    // Next attempt should fail fast without executing
    let before_count = attempt_count.load(Ordering::SeqCst);

    let attempt_count_clone = Arc::clone(&attempt_count);
    let result = circuit_breaker
        .execute(|| {
            let attempt_count = Arc::clone(&attempt_count_clone);
            async move {
                attempt_count.fetch_add(1, Ordering::SeqCst);
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ResilienceError::CircuitBreakerOpen { .. }
    ));

    // Operation should not have been executed
    assert_eq!(attempt_count.load(Ordering::SeqCst), before_count);
}

/// Test: Timeout with Bulkhead
/// Scenario: Multiple operations, some timeout, bulkhead limits concurrency
#[tokio::test]
async fn test_timeout_with_bulkhead() {
    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 2,
        queue_size: 5,
        timeout: Some(Duration::from_millis(100)),
    }));

    let success_count = Arc::new(AtomicU32::new(0));
    let mut handles = vec![];

    // Spawn 5 operations (bulkhead allows max 2 concurrent)
    for i in 0..5 {
        let bulkhead = Arc::clone(&bulkhead);
        let success_count = Arc::clone(&success_count);

        let handle = tokio::spawn(async move {
            let result = bulkhead
                .execute(|| async move {
                    // Some operations take longer
                    if i < 2 {
                        sleep(Duration::from_millis(50)).await;
                    } else {
                        sleep(Duration::from_millis(150)).await; // Will timeout
                    }
                    Ok::<_, ResilienceError>(i)
                })
                .await;

            if result.is_ok() {
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            result
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // First 2 should succeed (fast operations)
    assert!(results[0].as_ref().unwrap().is_ok());
    assert!(results[1].as_ref().unwrap().is_ok());

    // Later ones should timeout or succeed based on timing
    let success_count = success_count.load(Ordering::SeqCst);
    assert!(success_count >= 2, "Expected at least 2 successes");
}

/// Test: Full policy composition using PolicyBuilder
#[tokio::test]
async fn test_full_policy_composition() {
    let manager = Arc::new(ResilienceManager::with_defaults());

    // Register comprehensive policy using new API
    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(2))
        .with_retry_exponential(3, Duration::from_millis(100))
        .with_bulkhead(BulkheadConfig {
            max_concurrency: 10,
            queue_size: 20,
            timeout: Some(Duration::from_secs(5)),
        })
        .build();

    manager.register_service("test-service", policy);

    let attempt_count = Arc::new(AtomicU32::new(0));

    // Execute operation that fails once then succeeds
    let attempt_count_clone = Arc::clone(&attempt_count);
    let result = manager
        .execute("test-service", "test-operation", move || {
            let attempt_count = Arc::clone(&attempt_count_clone);
            async move {
                let count = attempt_count.fetch_add(1, Ordering::SeqCst);

                if count == 0 {
                    // First attempt fails
                    Err(ResilienceError::custom("First attempt failure"))
                } else {
                    // Retry succeeds
                    Ok("Success after retry")
                }
            }
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Success after retry");

    // Should have attempted twice (1 failure + 1 retry)
    assert_eq!(attempt_count.load(Ordering::SeqCst), 2);

    // Verify metrics are available
    let metrics = manager.get_metrics("test-service").await;
    assert!(metrics.is_some());

    let metrics = metrics.unwrap();
    // Verify bulkhead stats are present
    assert!(metrics.bulkhead.is_some());
}

/// Test: Concurrent access to ResilienceManager
#[tokio::test]
async fn test_manager_concurrent_access() {
    let manager = Arc::new(ResilienceManager::with_defaults());

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_millis(500))
        .build();

    manager.register_service("concurrent-test", policy);

    let mut handles = vec![];
    let success_count = Arc::new(AtomicU32::new(0));

    // Spawn 50 concurrent operations
    for i in 0..50 {
        let manager = Arc::clone(&manager);
        let success_count = Arc::clone(&success_count);

        let handle = tokio::spawn(async move {
            let result = manager
                .execute("concurrent-test", "operation", || async move {
                    sleep(Duration::from_millis(10)).await;
                    Ok::<_, ResilienceError>(i)
                })
                .await;

            if result.is_ok() {
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            result
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // All operations should succeed
    for result in results {
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    assert_eq!(success_count.load(Ordering::SeqCst), 50);
}

/// Test: Failure recovery scenario
/// Circuit opens, waits for reset, recovers
#[tokio::test]
async fn test_failure_recovery_scenario() {
    // Circuit breaker with 2 failure threshold, 200ms reset
    // Set min_operations to 1 so circuit opens as soon as failure threshold is reached
    // Set half_open_limit to 1 so one success closes the circuit
    let config = CircuitBreakerConfig::<2, 200>::new()
        .with_min_operations(1)
        .with_half_open_limit(1);
    let circuit_breaker = Arc::new(CircuitBreaker::new(config).unwrap());

    // Phase 1: Cause failures to open circuit
    for _ in 0..2 {
        let _ = circuit_breaker
            .execute(|| async { Err::<(), _>(ResilienceError::custom("Failure")) })
            .await;
    }

    let stats = circuit_breaker.stats().await;
    assert_eq!(
        stats.state,
        nebula_resilience::patterns::circuit_breaker::State::Open
    );

    // Phase 2: Wait for reset timeout
    sleep(Duration::from_millis(250)).await;

    // Phase 3: Circuit should be half-open, try recovery
    let result = circuit_breaker
        .execute(|| async { Ok::<_, ResilienceError>("Recovered") })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Recovered");

    // Phase 4: Circuit should be closed again
    let stats = circuit_breaker.stats().await;
    assert_eq!(
        stats.state,
        nebula_resilience::patterns::circuit_breaker::State::Closed
    );
}
