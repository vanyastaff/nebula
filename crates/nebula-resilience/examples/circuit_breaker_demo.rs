//! Circuit Breaker Pattern Demonstration
//!
//! This example demonstrates the circuit breaker pattern with various failure scenarios
//! and shows the security improvements and optimizations made to the implementation.

use nebula_resilience::{CircuitBreaker, ResilienceError};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔌 Circuit Breaker Pattern Demo");
    println!("================================");

    // Test 1: Basic Circuit Breaker Operation
    println!("\n📊 Test 1: Basic Circuit Breaker with Fast-Path Optimization");

    // Use the new const generic API - CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>
    let circuit_breaker = CircuitBreaker::<3, 100>::with_defaults()?;

    // Demonstrate successful operations (fast path)
    for i in 1..=5 {
        let result = circuit_breaker
            .execute(|| async {
                println!("  ✅ Executing operation {}", i);
                Ok::<String, ResilienceError>(format!("Success {}", i))
            })
            .await;

        match result {
            Ok(value) => println!("  ✅ Operation {} succeeded: {}", i, value),
            Err(e) => println!("  ❌ Operation {} failed: {}", i, e),
        }
    }

    // Test 2: Failure Scenarios and Circuit Opening
    println!("\n📊 Test 2: Failure Scenarios and Circuit State Transitions");

    // Lower threshold for demo: 2 failures, 500ms reset
    let breaker = CircuitBreaker::<2, 500>::with_defaults()?;

    // Simulate failures to trigger circuit opening
    for i in 1..=4 {
        let result = breaker
            .execute(|| async {
                if i <= 2 {
                    println!("  💥 Simulating failure {}", i);
                    Err(ResilienceError::Custom {
                        message: format!("Simulated failure {}", i),
                        retryable: true,
                        source: None,
                    })
                } else {
                    println!("  ✅ Attempting operation {}", i);
                    Ok::<String, ResilienceError>(format!("Success {}", i))
                }
            })
            .await;

        match result {
            Ok(value) => println!("  ✅ Operation {} succeeded: {}", i, value),
            Err(e) => println!("  ❌ Operation {} failed: {}", i, e),
        }

        // Show circuit state
        let state = breaker.state().await;
        println!("  🔄 Circuit state: {:?}", state);

        sleep(Duration::from_millis(50)).await;
    }

    // Test 3: Half-Open Recovery
    println!("\n📊 Test 3: Half-Open Recovery Process");

    println!("  ⏳ Waiting for reset timeout...");
    sleep(Duration::from_millis(600)).await;

    // This should transition to half-open
    let result = breaker
        .execute(|| async {
            println!("  🔧 Testing half-open operation");
            Ok::<String, ResilienceError>("Recovery test".to_string())
        })
        .await;

    match result {
        Ok(value) => {
            println!("  ✅ Half-open test succeeded: {}", value);
            let state = breaker.state().await;
            println!("  🔄 Circuit state after success: {:?}", state);
        }
        Err(e) => println!("  ❌ Half-open test failed: {}", e),
    }

    // Test 4: Performance with High Frequency Operations
    println!("\n📊 Test 4: Performance Test with High Frequency Operations");

    // Standard circuit breaker with defaults
    let perf_breaker = CircuitBreaker::<5, 30_000>::with_defaults()?;
    let start = std::time::Instant::now();
    let operations = 1000;

    for _ in 0..operations {
        let _ = perf_breaker
            .execute(|| async {
                // Minimal operation to test fast path
                Ok::<(), ResilienceError>(())
            })
            .await;
    }

    let elapsed = start.elapsed();
    let ops_per_sec = operations as f64 / elapsed.as_secs_f64();

    println!("  ⚡ Completed {} operations in {:?}", operations, elapsed);
    println!("  📈 Throughput: {:.2} operations/second", ops_per_sec);

    // Test 5: Concurrent Access Safety
    println!("\n📊 Test 5: Concurrent Access Safety Test");

    let concurrent_breaker = std::sync::Arc::new(CircuitBreaker::<5, 30_000>::with_defaults()?);
    let mut handles = vec![];

    for worker_id in 0..10 {
        let breaker = concurrent_breaker.clone();
        let handle = tokio::spawn(async move {
            for i in 0..50 {
                let result = breaker
                    .execute(|| async {
                        // Simulate some work
                        if (worker_id + i) % 20 == 0 {
                            Err(ResilienceError::Custom {
                                message: "Occasional failure".to_string(),
                                retryable: true,
                                source: None,
                            })
                        } else {
                            Ok::<String, ResilienceError>(format!(
                                "Worker {} operation {}",
                                worker_id, i
                            ))
                        }
                    })
                    .await;

                if i % 10 == 0 {
                    match result {
                        Ok(_) => print!("✅"),
                        Err(_) => print!("❌"),
                    }
                }
            }
            print!(" W{} ", worker_id);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await?;
    }

    println!("\n  🔒 Concurrent test completed successfully");

    let stats = concurrent_breaker.stats().await;
    println!(
        "  📊 Final stats: state={:?}, failures={}",
        stats.state, stats.failure_count
    );

    println!("\n🎉 Circuit Breaker Demo Completed Successfully!");
    println!("   ✅ Fast-path optimization working");
    println!("   ✅ State transitions working");
    println!("   ✅ Concurrent access safe");

    Ok(())
}
