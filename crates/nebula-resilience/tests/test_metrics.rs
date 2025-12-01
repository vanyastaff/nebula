//! Integration tests for metrics collection

use nebula_resilience::prelude::*;
use std::time::Duration;

#[tokio::test]
async fn test_get_metrics_for_registered_service() {
    let manager = ResilienceManager::with_defaults();

    // Register a service with default policy
    let policy = PolicyBuilder::new().build();
    manager.register_service("test-api", policy).await;

    // Get metrics
    let metrics = manager.get_metrics("test-api").await;

    assert!(metrics.is_some());
    let metrics = metrics.unwrap();
    assert_eq!(metrics.service_name, "test-api");
}

#[tokio::test]
async fn test_get_metrics_for_unregistered_service() {
    let manager = ResilienceManager::with_defaults();

    // Try to get metrics for non-existent service
    let metrics = manager.get_metrics("nonexistent").await;

    assert!(metrics.is_none());
}

#[tokio::test]
async fn test_circuit_breaker_metrics() {
    let manager = ResilienceManager::with_defaults();

    // Register service with circuit breaker config
    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(5))
        .with_retry_fixed(3, Duration::from_millis(100))
        .with_circuit_breaker(CircuitBreakerConfig::default())
        .build();
    manager.register_service("api-with-cb", policy).await;

    // Get metrics
    let metrics = manager.get_metrics("api-with-cb").await;

    assert!(metrics.is_some());
    let metrics = metrics.unwrap();

    // Should have circuit breaker stats
    assert!(metrics.circuit_breaker.is_some());
    let cb_stats = metrics.circuit_breaker.unwrap();

    // Initial state should be Closed
    assert_eq!(format!("{:?}", cb_stats.state), "Closed");
    assert_eq!(cb_stats.failure_count, 0);
}

#[tokio::test]
async fn test_get_all_metrics() {
    let manager = ResilienceManager::with_defaults();

    // Register multiple services
    let policy = PolicyBuilder::new().build();
    manager.register_service("api1", policy.clone()).await;
    manager.register_service("api2", policy.clone()).await;
    manager.register_service("api3", policy).await;

    // Get all metrics
    let all_metrics = manager.get_all_metrics().await;

    assert_eq!(all_metrics.len(), 3);
    assert!(all_metrics.contains_key("api1"));
    assert!(all_metrics.contains_key("api2"));
    assert!(all_metrics.contains_key("api3"));
}

#[tokio::test]
async fn test_metrics_after_service_unregister() {
    let manager = ResilienceManager::with_defaults();

    let policy = PolicyBuilder::new().build();
    manager.register_service("temp-api", policy).await;

    // Verify service exists
    assert!(manager.get_metrics("temp-api").await.is_some());

    // Unregister service
    manager.unregister_service("temp-api").await;

    // Metrics should return None
    assert!(manager.get_metrics("temp-api").await.is_none());
}
