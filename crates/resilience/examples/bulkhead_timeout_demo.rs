//! Bulkhead and Timeout Pattern Demonstration
//!
//! This example demonstrates bulkhead isolation and timeout patterns with the
//! performance optimizations and security improvements.

use nebula_resilience::{Bulkhead, ResilienceError, timeout_fn, timeout_with_original_error};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🏗️  Bulkhead and Timeout Pattern Demo");
    println!("====================================");

    // Test 1: Basic Bulkhead Operation
    println!("\n📊 Test 1: Basic Bulkhead with Performance Optimizations");

    let bulkhead = Bulkhead::new(3); // Allow 3 concurrent operations
    println!("  ✅ Created bulkhead with capacity 3");

    // Test sequential operations (should all succeed quickly)
    println!("  🧪 Testing sequential operations...");
    for i in 1..=5 {
        let start = Instant::now();
        let result = bulkhead
            .execute(|| async {
                sleep(Duration::from_millis(50)).await;
                Ok::<String, ResilienceError>(format!("Operation {}", i))
            })
            .await;

        match result {
            Ok(value) => println!("    ✅ {} completed in {:?}", value, start.elapsed()),
            Err(e) => println!("    ❌ Operation {} failed: {}", i, e),
        }
    }

    // Test 2: Concurrent Operations and Resource Isolation
    println!("\n📊 Test 2: Concurrent Operations and Resource Isolation");

    let concurrent_bulkhead = Arc::new(Bulkhead::new(2));
    let mut handles = vec![];

    println!("  🧪 Starting 5 concurrent operations (bulkhead capacity = 2)...");

    for i in 1..=5 {
        let bulkhead = concurrent_bulkhead.clone();
        let handle = tokio::spawn(async move {
            let start = Instant::now();
            let result = bulkhead
                .execute(|| async {
                    println!("    🔄 Operation {} started", i);
                    sleep(Duration::from_millis(300)).await;
                    println!("    ✅ Operation {} completed", i);
                    Ok::<String, ResilienceError>(format!("Concurrent operation {}", i))
                })
                .await;

            match result {
                Ok(value) => println!("    📋 {} finished in {:?}", value, start.elapsed()),
                Err(e) => println!("    ❌ Operation {} error: {}", i, e),
            }
        });
        handles.push(handle);

        // Small delay between starts to see the queuing effect
        sleep(Duration::from_millis(50)).await;
    }

    // Wait for all operations to complete
    for handle in handles {
        handle.await?;
    }

    // Test 3: Bulkhead Statistics and Monitoring
    println!("\n📊 Test 3: Bulkhead Statistics and Monitoring");

    let stats_bulkhead = Bulkhead::new(2);
    println!("  ✅ Created bulkhead for statistics testing");

    // Show initial stats
    let stats = stats_bulkhead.stats();
    println!(
        "  📊 Initial stats: max={}, active={}, available={}, at_capacity={}",
        stats.max_concurrency,
        stats.active_operations,
        stats.available_permits,
        stats.is_at_capacity
    );

    // Start a long-running operation
    let stats_bulkhead_clone = stats_bulkhead.clone();
    let long_running = tokio::spawn(async move {
        let _permit = stats_bulkhead_clone.acquire().await.unwrap();
        println!("    🔄 Long-running operation holding permit...");
        sleep(Duration::from_secs(1)).await;
        println!("    ✅ Long-running operation completed");
    });

    // Give it time to acquire the permit
    sleep(Duration::from_millis(50)).await;

    // Show stats with active operation
    let stats = stats_bulkhead.stats();
    println!(
        "  📊 With active operation: max={}, active={}, available={}, at_capacity={}",
        stats.max_concurrency,
        stats.active_operations,
        stats.available_permits,
        stats.is_at_capacity
    );

    long_running.await?;

    // Test 4: Timeout Pattern Demonstration
    println!("\n📊 Test 4: Timeout Pattern with Error Handling");

    // Test successful operation within timeout
    println!("  🧪 Testing successful operation within timeout...");
    let result = timeout_fn(Duration::from_millis(200), async {
        sleep(Duration::from_millis(100)).await;
        Ok::<String, ResilienceError>("Fast operation".to_string())
    })
    .await;

    match result {
        Ok(Ok(value)) => println!("    ✅ Operation succeeded: {}", value),
        Ok(Err(e)) => println!("    ❌ Operation failed: {}", e),
        Err(_) => println!("    ⏰ Operation timed out"),
    }

    // Test operation that times out
    println!("  🧪 Testing operation that times out...");
    let result = timeout_fn(Duration::from_millis(100), async {
        sleep(Duration::from_millis(200)).await;
        Ok::<String, ResilienceError>("Slow operation".to_string())
    })
    .await;

    match result {
        Ok(Ok(value)) => println!("    ✅ Operation succeeded: {}", value),
        Ok(Err(e)) => println!("    ❌ Operation failed: {}", e),
        Err(_) => println!("    ⏰ Operation timed out as expected"),
    }

    // Test timeout with original error preservation
    println!("  🧪 Testing timeout with original error preservation...");
    let result = timeout_with_original_error(Duration::from_millis(100), async {
        sleep(Duration::from_millis(50)).await;
        Err::<String, ResilienceError>(ResilienceError::Custom {
            message: "Original error".to_string(),
            retryable: true,
            source: None,
        })
    })
    .await;

    match result {
        Ok(value) => println!("    ✅ Operation succeeded: {}", value),
        Err(e) => println!("    ❌ Original error preserved: {}", e),
    }

    // Test 5: Combined Bulkhead + Timeout
    println!("\n📊 Test 5: Combined Bulkhead and Timeout Pattern");

    let combined_bulkhead = Bulkhead::new(1);

    // Start a blocking operation
    let blocking_bulkhead = combined_bulkhead.clone();
    let blocking_task = tokio::spawn(async move {
        blocking_bulkhead
            .execute(|| async {
                println!("    🔒 Blocking operation started");
                sleep(Duration::from_secs(1)).await;
                println!("    🔓 Blocking operation completed");
                Ok::<String, ResilienceError>("Blocking operation".to_string())
            })
            .await
    });

    // Give blocking operation time to start
    sleep(Duration::from_millis(50)).await;

    // Try another operation with timeout
    println!("  🧪 Testing bulkhead timeout interaction...");
    let timeout_result = tokio::time::timeout(
        Duration::from_millis(200),
        combined_bulkhead.execute(|| async {
            Ok::<String, ResilienceError>("Should timeout waiting for permit".to_string())
        }),
    )
    .await;

    match timeout_result {
        Ok(Ok(value)) => println!("    ✅ Unexpected success: {}", value),
        Ok(Err(e)) => println!("    ❌ Operation error: {}", e),
        Err(_) => println!("    ⏰ Timed out waiting for bulkhead permit (expected)"),
    }

    blocking_task.await??;

    // Test 6: Performance Under Load
    println!("\n📊 Test 6: Performance Under Load");

    let perf_bulkhead = Arc::new(Bulkhead::new(10));
    let operations = 100;
    let start = Instant::now();

    let mut perf_handles = vec![];
    for i in 0..operations {
        let bulkhead = perf_bulkhead.clone();
        let handle = tokio::spawn(async move {
            bulkhead
                .execute(|| async {
                    // Minimal work to test overhead
                    sleep(Duration::from_millis(1)).await;
                    Ok::<usize, ResilienceError>(i)
                })
                .await
        });
        perf_handles.push(handle);
    }

    let mut successful = 0;
    let mut failed = 0;

    for handle in perf_handles {
        match handle.await? {
            Ok(_) => successful += 1,
            Err(_) => failed += 1,
        }
    }

    let elapsed = start.elapsed();
    let throughput = operations as f64 / elapsed.as_secs_f64();

    println!("  ⚡ Completed {} operations in {:?}", operations, elapsed);
    println!("  📊 Results: {} successful, {} failed", successful, failed);
    println!("  📈 Throughput: {:.2} operations/second", throughput);

    // Test 7: Error Scenarios
    println!("\n📊 Test 7: Error Handling Scenarios");

    let error_bulkhead = Bulkhead::new(2);

    // Test operation that fails
    let result = error_bulkhead
        .execute(|| async {
            Err::<String, ResilienceError>(ResilienceError::Custom {
                message: "Simulated failure".to_string(),
                retryable: true,
                source: None,
            })
        })
        .await;

    match result {
        Ok(_) => println!("  ❌ Unexpected success"),
        Err(e) => println!("  ✅ Error handled correctly: {}", e),
    }

    // Test timeout within bulkhead operation
    let result = error_bulkhead
        .execute_with_timeout(Duration::from_millis(50), || async {
            sleep(Duration::from_millis(100)).await;
            Ok::<String, ResilienceError>("Should timeout".to_string())
        })
        .await;

    match result {
        Ok(_) => println!("  ❌ Unexpected success"),
        Err(ResilienceError::Timeout { .. }) => println!("  ✅ Timeout handled correctly"),
        Err(e) => println!("  ❌ Unexpected error: {}", e),
    }

    println!("\n🎉 Bulkhead and Timeout Demo Completed Successfully!");
    println!("   ✅ Bulkhead isolation working");
    println!("   ✅ Performance optimizations active");
    println!("   ✅ Timeout patterns working");
    println!("   ✅ Combined patterns working");
    println!("   ✅ Error handling robust");

    Ok(())
}
