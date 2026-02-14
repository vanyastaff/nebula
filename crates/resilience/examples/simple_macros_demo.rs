//! Simple demonstration of helper macros for cleaner resilience code

use std::time::Duration;
use tokio::time::sleep;

use nebula_resilience::prelude::*;
use nebula_resilience::{log_result, print_result};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    nebula_log::auto_init()?;

    info!("Starting simple macros demonstration");

    // Example 1: Using log_result macro for structured logging
    println!("\n=== Example 1: log_result macro ===");

    let operation_result = simulate_operation(true).await;
    log_result!(
        operation_result,
        "successful_operation",
        "Simulated successful operation"
    );

    let failed_result = simulate_operation(false).await;
    log_result!(
        failed_result,
        "failed_operation",
        "Simulated failed operation"
    );

    // Example 2: Using print_result macro for console output
    println!("\n=== Example 2: print_result macro ===");

    let result1 = simulate_operation(true).await;
    print_result!(
        result1,
        "Operation 1 completed successfully with result: {:?}",
        result1
    );

    let result2 = simulate_operation(false).await;
    print_result!(result2, "Operation 2 attempted");

    // Example 3: Short notation for quick indicators
    println!("\n=== Example 3: Quick indicators ===");
    print!("Running 10 operations: ");
    for i in 1..=10 {
        let result = simulate_operation(i % 3 != 0).await;
        print_result!(short result);
    }
    println!();

    // Example 4: Using with actual resilience patterns
    println!("\n=== Example 4: With CircuitBreaker ===");

    // Use StandardCircuitBreaker which is a type alias for CircuitBreaker<5, 30_000>
    let circuit_breaker = StandardCircuitBreaker::default();

    for i in 1..=5 {
        let result = circuit_breaker
            .execute(|| async { simulate_operation(i % 2 == 0).await })
            .await;

        print_result!(result, "CircuitBreaker operation {} result", i);
    }

    info!("Simple macros demonstration completed");
    Ok(())
}

/// Simulate an operation that can succeed or fail
async fn simulate_operation(should_succeed: bool) -> Result<String, ResilienceError> {
    sleep(Duration::from_millis(10)).await;

    if should_succeed {
        Ok("Operation completed successfully".to_string())
    } else {
        Err(ResilienceError::Custom {
            message: "Operation failed".to_string(),
            retryable: true,
            source: None,
        })
    }
}
