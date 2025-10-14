//! Integration tests for concurrent access scenarios
//!
//! Tests thread-safety and concurrent operation handling:
//! - Bulkhead concurrency limits
//! - Rate limiter under load
//! - Manager concurrent operations
//! - Race condition tests

use nebula_resilience::prelude::*;
use nebula_resilience::{RateLimiter, TokenBucket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::time::sleep;

/// Test: Bulkhead enforces concurrency limit
#[tokio::test]
async fn test_bulkhead_concurrency_limit() {
    let bulkhead = Arc::new(Bulkhead::new(3)); // Max 3 concurrent
    let active_count = Arc::new(AtomicU32::new(0));
    let max_observed = Arc::new(AtomicU32::new(0));

    let mut handles = vec![];

    // Spawn 10 operations
    for _ in 0..10 {
        let bulkhead = Arc::clone(&bulkhead);
        let active_count = Arc::clone(&active_count);
        let max_observed = Arc::clone(&max_observed);

        let handle = tokio::spawn(async move {
            bulkhead
                .execute(|| async move {
                    // Track active operations
                    let current = active_count.fetch_add(1, Ordering::SeqCst) + 1;

                    // Update max observed
                    max_observed.fetch_max(current, Ordering::SeqCst);

                    // Simulate work
                    sleep(Duration::from_millis(100)).await;

                    // Decrement active count
                    active_count.fetch_sub(1, Ordering::SeqCst);

                    Ok::<_, ResilienceError>(())
                })
                .await
        });

        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    // Max concurrent should never exceed bulkhead limit
    let max = max_observed.load(Ordering::SeqCst);
    assert!(
        max <= 3,
        "Bulkhead limit violated: max concurrent was {}",
        max
    );
}

/// Test: Rate limiter under high load
#[tokio::test]
async fn test_rate_limiter_high_load() {
    let rate_limiter = Arc::new(TokenBucket::new(10, 10.0)); // 10 ops/sec
    let success_count = Arc::new(AtomicU32::new(0));
    let rejected_count = Arc::new(AtomicU32::new(0));

    let mut handles = vec![];

    // Try to execute 50 operations rapidly
    for _ in 0..50 {
        let rate_limiter = Arc::clone(&rate_limiter);
        let success_count = Arc::clone(&success_count);
        let rejected_count = Arc::clone(&rejected_count);

        let handle = tokio::spawn(async move {
            match rate_limiter.acquire().await {
                Ok(_) => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    rejected_count.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    let successes = success_count.load(Ordering::SeqCst);
    let rejections = rejected_count.load(Ordering::SeqCst);

    // Should allow ~10 operations (bucket capacity)
    assert!(
        successes >= 10 && successes <= 15,
        "Expected ~10 successes, got {}",
        successes
    );

    // Rest should be rejected
    assert!(rejections >= 35, "Expected rejections, got {}", rejections);

    assert_eq!(successes + rejections, 50);
}

/// Test: Manager handles concurrent service registration
#[tokio::test]
async fn test_manager_concurrent_registration() {
    let manager = Arc::new(ResilienceManager::with_defaults());
    let mut handles = vec![];

    // Concurrently register 20 services
    for i in 0..20 {
        let manager = Arc::clone(&manager);

        let handle = tokio::spawn(async move {
            let service_name = format!("service-{}", i);
            let policy = ResiliencePolicy::default().with_timeout(Duration::from_secs(1));

            manager.register_service(&service_name, policy).await;
            service_name
        });

        handles.push(handle);
    }

    let service_names: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All services should be registered
    let registered_services = manager.list_services();
    assert_eq!(registered_services.len(), 20);

    // Verify each service is registered
    for service_name in service_names {
        assert!(registered_services.contains(&service_name));
    }
}

/// Test: Concurrent operations don't interfere with each other
#[tokio::test]
async fn test_manager_operation_isolation() {
    let manager = Arc::new(ResilienceManager::with_defaults());

    let policy = ResiliencePolicy::default()
        .with_timeout(Duration::from_secs(1))
        .with_bulkhead(BulkheadConfig {
            max_concurrency: 5,
            queue_size: 10,
            timeout: None,
        });

    manager.register_service("isolated-service", policy).await;

    let mut handles = vec![];

    // Spawn 20 concurrent operations with different delays
    for i in 0..20 {
        let manager = Arc::clone(&manager);

        let handle = tokio::spawn(async move {
            manager
                .execute("isolated-service", "operation", || async move {
                    // Variable delays
                    let delay = Duration::from_millis(50 + (i % 5) * 10);
                    sleep(delay).await;
                    Ok::<_, ResilienceError>(i)
                })
                .await
        });

        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // All operations should succeed
    let mut values: Vec<u64> = results.into_iter().map(|r| r.unwrap().unwrap()).collect();

    values.sort();

    // Should have all values 0..20
    assert_eq!(values.len(), 20);
    for (i, &value) in values.iter().enumerate() {
        assert_eq!(value, i as u64);
    }
}

/// Test: Circuit breaker state transitions under concurrent load
#[tokio::test]
async fn test_circuit_breaker_concurrent_transitions() {
    let circuit_breaker = Arc::new(CircuitBreaker::with_config(CircuitBreakerConfig {
        failure_threshold: 5,
        reset_timeout: Duration::from_millis(500),
        half_open_max_operations: 2,
        count_timeouts: false,
    }));

    let failure_count = Arc::new(AtomicU32::new(0));

    // Phase 1: Concurrent failures to open circuit
    let mut handles = vec![];
    for _ in 0..10 {
        let cb = Arc::clone(&circuit_breaker);
        let failure_count = Arc::clone(&failure_count);

        let handle = tokio::spawn(async move {
            let result = cb
                .execute(|| async {
                    failure_count.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(ResilienceError::custom("Concurrent failure"))
                })
                .await;
            result
        });

        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    // Circuit should be open
    let stats = circuit_breaker.stats().await;
    assert_eq!(stats.state, CircuitState::Open);

    // Phase 2: Wait for reset
    sleep(Duration::from_millis(600)).await;

    // Phase 3: Concurrent recovery attempts
    let mut handles = vec![];
    let success_count = Arc::new(AtomicU32::new(0));

    for _ in 0..5 {
        let cb = Arc::clone(&circuit_breaker);
        let success_count = Arc::clone(&success_count);

        let handle = tokio::spawn(async move {
            let result = cb
                .execute(|| async { Ok::<_, ResilienceError>("Success") })
                .await;

            if result.is_ok() {
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            result
        });

        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    // At least some should succeed (half-open allows limited ops)
    let successes = success_count.load(Ordering::SeqCst);
    assert!(successes > 0, "Expected some successful operations");
}

/// Test: Bulkhead with timeout under load
#[tokio::test]
async fn test_bulkhead_timeout_under_load() {
    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 2,
        queue_size: 3,
        timeout: Some(Duration::from_millis(100)),
    }));

    let mut handles = vec![];
    let timeout_count = Arc::new(AtomicU32::new(0));
    let success_count = Arc::new(AtomicU32::new(0));

    // Spawn 10 operations (only 2+3 can proceed, rest should be rejected)
    for i in 0..10 {
        let bulkhead = Arc::clone(&bulkhead);
        let timeout_count = Arc::clone(&timeout_count);
        let success_count = Arc::clone(&success_count);

        let handle = tokio::spawn(async move {
            let result = bulkhead
                .execute(|| async move {
                    // Slow operation
                    sleep(Duration::from_millis(50)).await;
                    Ok::<_, ResilienceError>(i)
                })
                .await;

            match result {
                Ok(_) => {
                    success_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(ResilienceError::BulkheadFull { .. }) => {
                    timeout_count.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {}
            }
        });

        handles.push(handle);
        sleep(Duration::from_millis(5)).await; // Small delay between spawns
    }

    futures::future::join_all(handles).await;

    let successes = success_count.load(Ordering::SeqCst);
    let timeouts = timeout_count.load(Ordering::SeqCst);

    // Should have some successes and some rejections
    assert!(successes > 0, "Expected some successes");
    // With slow operations and queue, most should succeed
    assert!(successes >= 5, "Expected at least half to succeed");
    assert_eq!(successes + timeouts, 10);
}

/// Test: Multiple managers with same services (isolation test)
#[tokio::test]
async fn test_multiple_managers_isolation() {
    let manager1 = Arc::new(ResilienceManager::with_defaults());
    let manager2 = Arc::new(ResilienceManager::with_defaults());

    let policy1 = ResiliencePolicy::default().with_timeout(Duration::from_millis(100));
    let policy2 = ResiliencePolicy::default().with_timeout(Duration::from_millis(200));

    // Register same service name with different policies
    manager1.register_service("shared-name", policy1).await;
    manager2.register_service("shared-name", policy2).await;

    // Execute operations in parallel
    let (result1, result2) = tokio::join!(
        manager1.execute("shared-name", "op", || async {
            sleep(Duration::from_millis(150)).await; // Will timeout in manager1
            Ok::<_, ResilienceError>("result")
        }),
        manager2.execute("shared-name", "op", || async {
            sleep(Duration::from_millis(150)).await; // Will succeed in manager2
            Ok::<_, ResilienceError>("result")
        })
    );

    // Manager1 should timeout
    assert!(result1.is_err());

    // Manager2 should succeed
    assert!(result2.is_ok());
}
