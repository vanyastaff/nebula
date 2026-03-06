//! Integration tests for fault-injection scenarios across pattern combinations.
//!
//! Covers Phase 4 reliability matrix:
//! - retry + circuit breaker
//! - retry + timeout
//! - circuit breaker + timeout
//! - retry + circuit breaker + timeout

use nebula_resilience::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_fault_injection_retry_and_circuit_breaker_interplay() {
    let manager = ResilienceManager::with_defaults();

    let circuit_config = CircuitBreakerConfig::default()
        .with_min_operations(1)
        .with_half_open_limit(1);

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_millis(100))
        .with_retry_fixed(2, Duration::from_millis(1))
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_rate_threshold: 0.0,
            ..circuit_config
        })
        .build();

    manager.register_service("fi-retry-breaker", policy);

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_first = Arc::clone(&attempts);
    let first = manager
        .execute("fi-retry-breaker", "op", move || {
            let attempts = Arc::clone(&attempts_first);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ResilienceError::custom("injected failure"))
            }
        })
        .await;

    assert!(matches!(
        first,
        Err(ResilienceError::RetryLimitExceeded { .. })
    ));

    let attempts_second = Arc::clone(&attempts);
    let second = manager
        .execute("fi-retry-breaker", "op", move || {
            let attempts = Arc::clone(&attempts_second);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<(), _>(())
            }
        })
        .await;

    assert!(matches!(
        second,
        Err(ResilienceError::CircuitBreakerOpen { .. })
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_fault_injection_retry_and_timeout_interplay() {
    let manager = ResilienceManager::with_defaults();

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_millis(10))
        .with_retry_fixed(3, Duration::from_millis(1))
        .build();

    manager.register_service("fi-retry-timeout", policy);

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_clone = Arc::clone(&attempts);
    let result = manager
        .execute("fi-retry-timeout", "op", move || {
            let attempts = Arc::clone(&attempts_clone);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(30)).await;
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    match result {
        Err(ResilienceError::RetryLimitExceeded {
            attempts: max_attempts,
            last_error,
        }) => {
            assert_eq!(max_attempts, 3);
            assert!(matches!(
                last_error.as_deref(),
                Some(ResilienceError::Timeout { .. })
            ));
        }
        other => panic!("Expected retry-limit timeout failure, got: {other:?}"),
    }

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_fault_injection_circuit_breaker_and_timeout_interplay() {
    let manager = ResilienceManager::with_defaults();

    let circuit_config = CircuitBreakerConfig::default()
        .with_min_operations(1)
        .with_half_open_limit(1);

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_millis(10))
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_rate_threshold: 0.0,
            ..circuit_config
        })
        .build();

    manager.register_service("fi-breaker-timeout", policy);

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_first = Arc::clone(&attempts);
    let first = manager
        .execute("fi-breaker-timeout", "op", move || {
            let attempts = Arc::clone(&attempts_first);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(30)).await;
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    assert!(matches!(first, Err(ResilienceError::Timeout { .. })));

    let attempts_second = Arc::clone(&attempts);
    let second = manager
        .execute("fi-breaker-timeout", "op", move || {
            let attempts = Arc::clone(&attempts_second);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    assert!(matches!(
        second,
        Err(ResilienceError::CircuitBreakerOpen { .. })
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_fault_injection_retry_breaker_timeout_combined() {
    let manager = ResilienceManager::with_defaults();

    let circuit_config = CircuitBreakerConfig::default()
        .with_min_operations(1)
        .with_half_open_limit(1);

    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_millis(10))
        .with_retry_fixed(2, Duration::from_millis(1))
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_rate_threshold: 0.0,
            ..circuit_config
        })
        .build();

    manager.register_service("fi-combined", policy);

    let attempts = Arc::new(AtomicU32::new(0));
    let attempts_first = Arc::clone(&attempts);
    let first = manager
        .execute("fi-combined", "op", move || {
            let attempts = Arc::clone(&attempts_first);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(30)).await;
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    assert!(matches!(
        first,
        Err(ResilienceError::RetryLimitExceeded { .. })
    ));

    let attempts_second = Arc::clone(&attempts);
    let second = manager
        .execute("fi-combined", "op", move || {
            let attempts = Arc::clone(&attempts_second);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<(), ResilienceError>(())
            }
        })
        .await;

    assert!(matches!(
        second,
        Err(ResilienceError::CircuitBreakerOpen { .. })
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
