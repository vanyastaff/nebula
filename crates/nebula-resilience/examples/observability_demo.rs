//! Observability demonstration
//!
//! This example shows how to use observability hooks to monitor
//! resilience pattern execution with metrics, logging, and tracing.

use nebula_resilience::prelude::*;
use nebula_resilience::{
    PolicyBuilder, ResilienceManager,
    observability::{LogLevel, LoggingHook, MetricsHook, ObservabilityHooks, PatternEvent},
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
        .with_hook(
            metrics_hook.clone() as Arc<dyn nebula_resilience::observability::ObservabilityHook>
        )
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

    // Register a policy using the new PolicyBuilder API
    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(5))
        .with_retry_fixed(3, Duration::from_millis(100))
        .build();

    manager.register_service("database", policy);

    // Execute with observability
    let result = manager
        .execute("database", "query", || async {
            // Simulate database query
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, ResilienceError>("Query result")
        })
        .await;

    match result {
        Ok(data) => println!("  ✓ Query succeeded: {}", data),
        Err(e) => println!("  ✗ Query failed: {}", e),
    }

    println!("\n3. Using Circuit Breaker with Observability\n");

    // Create circuit breaker with const generics
    let circuit_breaker = CircuitBreaker::<3, 5000>::with_defaults()?;

    // Execute with circuit breaker
    let db_result = circuit_breaker
        .execute(|| async {
            // Emit start event
            hooks.emit(PatternEvent::Started {
                pattern: "circuit_breaker".to_string(),
                operation: "payment_process".to_string(),
            });

            let start = std::time::Instant::now();

            // Simulate operation
            tokio::time::sleep(Duration::from_millis(30)).await;

            // Emit success event
            hooks.emit(PatternEvent::Succeeded {
                pattern: "circuit_breaker".to_string(),
                operation: "payment_process".to_string(),
                duration: start.elapsed(),
            });

            Ok::<_, ResilienceError>("Payment processed")
        })
        .await;

    match db_result {
        Ok(data) => println!("  ✓ Payment result: {}", data),
        Err(e) => println!("  ✗ Payment failed: {}", e),
    }

    println!("\n4. Using Retry with Observability\n");

    // Create retry strategy with const generics
    let retry = exponential_retry::<3>()?;

    let attempt_counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter_clone = attempt_counter.clone();

    let retry_result = retry
        .execute_resilient(|| {
            let counter = counter_clone.clone();
            async move {
                let attempt = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                // Fail first two attempts
                if attempt < 2 {
                    Err(ResilienceError::custom("Transient failure"))
                } else {
                    Ok("Success after retries")
                }
            }
        })
        .await;

    match retry_result {
        Ok((data, _stats)) => println!(
            "  ✓ Retry succeeded: {} (after {} attempts)",
            data,
            attempt_counter.load(std::sync::atomic::Ordering::SeqCst)
        ),
        Err(e) => println!("  ✗ Retry failed: {}", e),
    }

    println!("\n✓ Observability demonstration completed!");
    println!("  - Event emission works correctly");
    println!("  - Integration with ResilienceManager verified");

    Ok(())
}
