//! Simple example demonstrating ResilienceManager usage
//!
//! This example shows:
//! - Creating a ResilienceManager with default policies
//! - Registering custom policies for specific services
//! - Executing operations through the manager
//! - Monitoring metrics
//!
//! Run with: cargo run --example simple_manager

use nebula_resilience::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== ResilienceManager Simple Example ===\n");

    // Create a manager with default policies (wrapped in Arc for cloning)
    let manager = Arc::new(ResilienceManager::with_defaults());

    // Example 1: Execute operation with default policy
    println!("1. Executing with default policy:");
    let result = manager
        .execute("unknown-service", "fetch_data", || async {
            println!("  -> Fetching data...");
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, ResilienceError>("Data fetched successfully")
        })
        .await?;
    println!("  ✓ Result: {}\n", result);

    // Example 2: Register a custom policy for a critical service
    println!("2. Registering custom policy for 'payment-api':");

    // Use PolicyBuilder for creating policies with the new API
    let payment_policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(5))
        .with_retry_exponential(3, Duration::from_millis(100))
        .with_bulkhead(BulkheadConfig {
            max_concurrency: 10,
            queue_size: 50,
            timeout: Some(Duration::from_secs(10)),
        })
        .build();

    manager
        .register_service("payment-api", payment_policy)
        .await;
    println!("  ✓ Custom policy registered\n");

    // Example 3: Execute operation with custom policy
    println!("3. Executing payment operation:");
    let payment_result = manager
        .execute("payment-api", "process_payment", || async {
            println!("  -> Processing payment...");
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<_, ResilienceError>("Payment processed: #12345")
        })
        .await?;
    println!("  ✓ {}\n", payment_result);

    // Example 4: Execute multiple operations concurrently
    println!("4. Executing multiple concurrent operations:");
    let mut handles = vec![];

    for i in 1..=5 {
        let manager_clone = Arc::clone(&manager);
        let handle = tokio::spawn(async move {
            manager_clone
                .execute("payment-api", "process_payment", || async move {
                    println!("  -> Processing payment {}...", i);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<_, ResilienceError>(format!("Payment {} completed", i))
                })
                .await
        });
        handles.push(handle);
    }

    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await? {
            Ok(result) => println!("  ✓ Payment {}: {}", i + 1, result),
            Err(e) => println!("  ✗ Payment {} failed: {}", i + 1, e),
        }
    }
    println!();

    // Example 5: Check metrics
    println!("5. Service metrics:");
    if let Some(metrics) = manager.get_metrics("payment-api").await {
        println!("  Total operations: {}", metrics.total_operations);
        println!("  Failed operations: {}", metrics.failed_operations);

        if let Some(cb_stats) = metrics.circuit_breaker {
            println!("\n  Circuit Breaker:");
            println!("    - State: {:?}", cb_stats.state);
            println!("    - Failure count: {}", cb_stats.failure_count);
            println!(
                "    - Half-open operations: {}",
                cb_stats.half_open_operations
            );
        }

        if let Some(bh_stats) = metrics.bulkhead {
            println!("\n  Bulkhead:");
            println!("    - Active operations: {}", bh_stats.active_operations);
            println!("    - Max concurrency: {}", bh_stats.max_concurrency);
        }
    }
    println!();

    // Example 6: List all registered services
    println!("6. Registered services:");
    for service in manager.list_services() {
        println!("  - {}", service);
    }

    println!("\n✓ Example completed successfully!");
    Ok(())
}
