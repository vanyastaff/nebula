//! Example demonstrating pattern composition and chaining
//!
//! This example shows how to combine multiple resilience patterns:
//! - Circuit Breaker + Retry
//! - Timeout + Bulkhead
//! - Rate Limiter + Circuit Breaker
//! - Complete policy with all patterns
//!
//! Run with: cargo run --example pattern_composition

use nebula_resilience::prelude::*;
use nebula_resilience::{retry, RateLimiter, TokenBucket};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Pattern Composition Examples ===\n");

    // Example 1: Circuit Breaker + Retry for Database Operations
    println!("1. Circuit Breaker + Retry (Database):");
    println!("   Protects against cascading failures with automatic retries\n");

    let circuit_breaker = CircuitBreaker::with_config(CircuitBreakerConfig {
        failure_threshold: 3,
        reset_timeout: Duration::from_secs(5),
        half_open_max_operations: 2,
        count_timeouts: true,
    });

    let retry_strategy = RetryStrategy::exponential_backoff(3, Duration::from_millis(100));

    for i in 1..=5 {
        let result = circuit_breaker
            .execute(|| {
                let strategy = retry_strategy.clone();
                async move {
                    retry(strategy, || async {
                        println!("  -> Attempt {} to connect to database", i);
                        tokio::time::sleep(Duration::from_millis(50)).await;

                        // Simulate intermittent failures
                        if i <= 2 {
                            println!("     Connection failed, will retry...");
                            Err(ResilienceError::custom("Connection refused"))
                        } else {
                            println!("     ✓ Connected successfully");
                            Ok::<_, ResilienceError>("Database query result")
                        }
                    })
                    .await
                }
            })
            .await;

        match result {
            Ok(data) => println!("  ✓ Operation {}: {}\n", i, data),
            Err(e) => println!("  ✗ Operation {} failed: {}\n", i, e),
        }
    }

    // Example 2: Timeout + Bulkhead for API Rate Control
    println!("2. Timeout + Bulkhead (API Rate Control):");
    println!("   Limits concurrent requests with timeouts\n");

    let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 3,
        queue_size: 5,
        timeout: Some(Duration::from_secs(2)),
    }));

    let mut handles = vec![];

    for i in 1..=8 {
        let bulkhead_clone = Arc::clone(&bulkhead);
        let handle = tokio::spawn(async move {
            println!("  -> Request {} queued", i);
            let result = bulkhead_clone
                .execute(|| async move {
                    println!("  -> Request {} executing", i);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok::<_, ResilienceError>(format!("Response {}", i))
                })
                .await;

            match &result {
                Ok(data) => println!("  ✓ {}", data),
                Err(e) => println!("  ✗ Request {} failed: {}", i, e),
            }
            result
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }
    println!();

    // Example 3: Rate Limiter + Circuit Breaker for External API
    println!("3. Rate Limiter + Circuit Breaker (External API):");
    println!("   Rate limits requests with circuit breaker protection\n");

    let rate_limiter = Arc::new(TokenBucket::new(2, 2.0)); // 2 requests per second
    let circuit_breaker = Arc::new(CircuitBreaker::with_config(CircuitBreakerConfig {
        failure_threshold: 2,
        reset_timeout: Duration::from_secs(3),
        half_open_max_operations: 1,
        count_timeouts: false,
    }));

    for i in 1..=6 {
        // First apply rate limiting
        match rate_limiter.acquire().await {
            Ok(_) => {
                // Then check circuit breaker
                let result = circuit_breaker
                    .execute(|| async move {
                        println!("  -> API call {}", i);
                        tokio::time::sleep(Duration::from_millis(50)).await;

                        // Simulate some failures
                        if i == 3 || i == 4 {
                            Err(ResilienceError::custom("API error 500"))
                        } else {
                            Ok::<_, ResilienceError>(format!("API response {}", i))
                        }
                    })
                    .await;

                match result {
                    Ok(data) => println!("  ✓ {}\n", data),
                    Err(e) => println!("  ✗ Call {} failed: {}\n", i, e),
                }
            }
            Err(e) => println!("  ✗ Rate limited: {}\n", e),
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Example 4: Complete Policy Composition
    println!("4. Complete Policy (All Patterns):");
    println!("   Demonstrates full ResiliencePolicy with all patterns\n");

    let complete_policy = ResiliencePolicy::default()
        .with_timeout(Duration::from_secs(5))
        .with_retry(RetryStrategy::fixed_delay(2, Duration::from_millis(100)))
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_secs(10),
            half_open_max_operations: 2,
            count_timeouts: true,
        })
        .with_bulkhead(BulkheadConfig {
            max_concurrency: 5,
            queue_size: 10,
            timeout: Some(Duration::from_secs(3)),
        });

    println!("  Policy configured with:");
    println!("    - Timeout: 5s");
    println!("    - Retry: Fixed delay (2 attempts, 100ms)");
    println!("    - Circuit Breaker: 3 failures threshold");
    println!("    - Bulkhead: 5 max concurrent, 10 queue size");
    println!("\n  Note: Use ResilienceManager to execute operations with this policy\n");

    // Example 5: Manual Composition - Circuit Breaker wrapping Bulkhead
    println!("5. Manual Composition:");
    println!("   Manually chaining Circuit Breaker -> Bulkhead\n");

    let result = circuit_breaker
        .execute(|| {
            let bulkhead_clone = Arc::clone(&bulkhead);
            async move {
                bulkhead_clone
                    .execute(|| async {
                        println!("  -> Executing composed operation");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok::<_, ResilienceError>("Success from composed operation")
                    })
                    .await
            }
        })
        .await;

    match result {
        Ok(data) => println!("  ✓ Composed result: {}\n", data),
        Err(e) => println!("  ✗ Composed operation failed: {}\n", e),
    }

    println!("✓ All composition examples completed!");
    Ok(())
}
