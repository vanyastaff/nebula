//! Observability demonstration
//!
//! This example shows how to use observability hooks to monitor
//! resilience pattern execution with metrics, logging, and tracing.

use nebula_resilience::{
    observability::{LogLevel, LoggingHook, MetricsHook, ObservabilityHooks, PatternEvent},
    CircuitBreaker, CircuitBreakerConfig, ResilienceError, ResilienceManager, ResiliencePolicy,
    RetryStrategy,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize nebula-log
    let _ = nebula_log::init();

    println!("=== Observability Demonstration ===\n");

    // Create observability hooks
    let metrics_hook = Arc::new(MetricsHook::new());
    let logging_hook = Arc::new(LoggingHook::new(LogLevel::Info));

    let hooks = ObservabilityHooks::new()
        .with_hook(metrics_hook.clone() as Arc<dyn nebula_resilience::observability::ObservabilityHook>)
        .with_hook(logging_hook);

    hooks.initialize();

    println!("1. Demonstrating event emission\n");

    // Example 1: Emit start event
    hooks.emit(PatternEvent::Started {
        pattern: "retry".to_string(),
        operation: "fetch_data".to_string(),
    });

    // Simulate some work
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Example 2: Emit success event
    hooks.emit(PatternEvent::Succeeded {
        pattern: "retry".to_string(),
        operation: "fetch_data".to_string(),
        duration: Duration::from_millis(50),
    });

    // Example 3: Circuit breaker state change
    hooks.emit(PatternEvent::CircuitBreakerStateChanged {
        service: "payment-api".to_string(),
        from_state: "closed".to_string(),
        to_state: "open".to_string(),
    });

    // Example 4: Rate limit exceeded
    hooks.emit(PatternEvent::RateLimitExceeded {
        service: "api".to_string(),
        current_rate: 150.0,
    });

    // Example 5: Bulkhead capacity reached
    hooks.emit(PatternEvent::BulkheadCapacityReached {
        service: "database".to_string(),
        active: 10,
        capacity: 10,
    });

    println!("\n2. Using with ResilienceManager\n");

    // Create manager with observability
    let manager = Arc::new(ResilienceManager::with_defaults());

    // Register a policy
    let policy = ResiliencePolicy::default()
        .with_timeout(Duration::from_secs(5))
        .with_retry(RetryStrategy::fixed_delay(3, Duration::from_millis(100)));

    manager.register_service("api", policy).await;

    // Execute operations and emit events
    for i in 1..=5 {
        hooks.emit(PatternEvent::Started {
            pattern: "manager".to_string(),
            operation: format!("api_call_{}", i),
        });

        let operation_name = format!("operation_{}", i);
        let result = manager
            .execute("api", &operation_name, || async {
                // Simulate work
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok::<_, ResilienceError>(format!("result_{}", i))
            })
            .await;

        match result {
            Ok(val) => {
                hooks.emit(PatternEvent::Succeeded {
                    pattern: "manager".to_string(),
                    operation: format!("api_call_{}", i),
                    duration: Duration::from_millis(10),
                });
                println!("✓ Operation {}: {}", i, val);
            }
            Err(e) => {
                hooks.emit(PatternEvent::Failed {
                    pattern: "manager".to_string(),
                    operation: format!("api_call_{}", i),
                    error: e.to_string(),
                    duration: Duration::from_millis(10),
                });
                println!("✗ Operation {} failed: {}", i, e);
            }
        }
    }

    println!("\n3. Metrics collected:\n");

    // Export and display metrics
    let metrics = metrics_hook.metrics();
    for (name, snapshot) in metrics.iter() {
        println!(
            "  {}: count={}, avg={:.2}ms, min={:.2}ms, max={:.2}ms",
            name, snapshot.count, snapshot.avg, snapshot.min, snapshot.max
        );
    }

    println!("\n4. Using spans for automatic timing\n");

    // Example with span guard
    use nebula_resilience::observability::SpanGuard;

    let span = SpanGuard::new("timeout", "database_query");
    tokio::time::sleep(Duration::from_millis(25)).await;
    span.success();

    println!("\n5. Circuit breaker with observability\n");

    let cb = Arc::new(CircuitBreaker::with_config(CircuitBreakerConfig {
        failure_threshold: 3,
        reset_timeout: Duration::from_secs(1),
        half_open_max_operations: 1,
        count_timeouts: true,
    }));

    // Simulate failures to trip circuit breaker
    for i in 1..=5 {
        let result: Result<(), ResilienceError> = cb
            .execute(|| async {
                if i < 4 {
                    Err::<(), _>(ResilienceError::custom("simulated failure"))
                } else {
                    Ok(())
                }
            })
            .await;

        match result {
            Ok(_) => {
                hooks.emit(PatternEvent::Succeeded {
                    pattern: "circuit_breaker".to_string(),
                    operation: format!("attempt_{}", i),
                    duration: Duration::from_millis(5),
                });
                println!("✓ Circuit breaker attempt {}: success", i);
            }
            Err(e) => {
                hooks.emit(PatternEvent::Failed {
                    pattern: "circuit_breaker".to_string(),
                    operation: format!("attempt_{}", i),
                    error: e.to_string(),
                    duration: Duration::from_millis(5),
                });
                println!("✗ Circuit breaker attempt {}: {}", i, e);
            }
        }
    }

    // Check circuit state
    let state = cb.state().await;
    println!("\nCircuit breaker state: {:?}", state);

    hooks.emit(PatternEvent::CircuitBreakerStateChanged {
        service: "test".to_string(),
        from_state: "closed".to_string(),
        to_state: format!("{:?}", state),
    });

    println!("\n6. Final metrics summary:\n");

    let final_metrics = metrics_hook.metrics();
    println!("Total metrics collected: {}", final_metrics.len());

    for (name, snapshot) in final_metrics.iter() {
        if snapshot.count > 0 {
            println!(
                "  {} -> {} events (avg: {:.2}ms)",
                name, snapshot.count, snapshot.avg
            );
        }
    }

    // Cleanup
    hooks.shutdown();

    println!("\n=== Demo completed successfully! ===");
    Ok(())
}
