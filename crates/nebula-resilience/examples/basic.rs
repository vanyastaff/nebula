//! Basic example demonstrating nebula-resilience usage

use nebula_resilience::{
    timeout, retry, circuit_breaker, bulkhead,
    ResiliencePolicy, ResilienceBuilder, policies
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    println!("=== Nebula Resilience Examples ===\n");

    // Example 1: Simple timeout
    println!("1. Simple Timeout Example");
    let result = timeout(
        Duration::from_millis(100),
        async { "operation completed" }
    ).await;
    println!("   Result: {:?}\n", result);

    // Example 2: Retry with exponential backoff
    println!("2. Retry Strategy Example");
    let retry_strategy = retry::RetryStrategy::new(3, Duration::from_millis(10))
        .with_jitter(0.1);
    
    let mut attempt_count = 0;
    let result = retry::retry(retry_strategy, || async {
        attempt_count += 1;
        if attempt_count < 3 {
            Err::<&str, &str>("temporary failure")
        } else {
            Ok("succeeded after retries")
        }
    }).await;
    println!("   Result: {:?}\n", result);

    // Example 3: Circuit Breaker
    println!("3. Circuit Breaker Example");
    let circuit_breaker = circuit_breaker::CircuitBreaker::new();
    
    // Simulate some failures to open the circuit
    for i in 0..6 {
        let result = circuit_breaker.execute(|| async {
            if i < 5 {
                Err(nebula_resilience::ResilienceError::timeout(Duration::from_secs(1)))
            } else {
                Ok("circuit breaker test")
            }
        }).await;
        println!("   Attempt {}: {:?}", i + 1, result);
    }
    println!();

    // Example 4: Bulkhead
    println!("4. Bulkhead Example");
    let bulkhead = bulkhead::Bulkhead::new(2);
    
    let mut handles = Vec::new();
    for i in 0..4 {
        let bulkhead = bulkhead.clone();
        let handle = tokio::spawn(async move {
            bulkhead.execute(|| async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<usize, nebula_resilience::ResilienceError>(i)
            }).await
        });
        handles.push(handle);
    }
    
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await??;
        println!("   Task {}: {:?}", i + 1, result);
    }
    println!();

    // Example 5: Resilience Policy
    println!("5. Resilience Policy Example");
    let policy = ResilienceBuilder::new()
        .timeout(Duration::from_millis(200))
        .retry(2, Duration::from_millis(10))
        .circuit_breaker(3, Duration::from_secs(1))
        .bulkhead(3)
        .build();
    
    let result = policy.execute(|| async {
        Ok::<&str, nebula_resilience::ResilienceError>("policy execution successful")
    }).await;
    println!("   Result: {:?}\n", result);

    // Example 6: Predefined Policies
    println!("6. Predefined Policies Example");
    println!("   Database policy: {:?}", policies::database());
    println!("   HTTP API policy: {:?}", policies::http_api());
    println!("   File operations policy: {:?}", policies::file_operations());
    println!("   Long running policy: {:?}", policies::long_running());
    println!("   Critical policy: {:?}", policies::critical());

    println!("\n=== Examples completed successfully! ===");
    Ok(())
}
