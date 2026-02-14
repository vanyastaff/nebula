//! Dynamic Configuration Builder Example
//!
//! This example demonstrates the type-safe builder pattern for creating
//! dynamic configurations at compile time.

use nebula_resilience::{ConfigError, core::DynamicConfigBuilder};
use std::time::Duration;

fn main() -> Result<(), ConfigError> {
    println!("=== Dynamic Configuration Builder Example ===\n");

    // Example 1: Simple retry configuration
    println!("1. Building retry configuration:");
    let retry_config = DynamicConfigBuilder::new()
        .retry()
        .max_attempts(3)
        .base_delay(Duration::from_millis(100))
        .done()?
        .build();

    println!(
        "   Max attempts: {:?}",
        retry_config.get_value("retry.max_attempts")?
    );
    println!(
        "   Base delay: {:?}",
        retry_config.get_value("retry.base_delay_ms")?
    );

    // Example 2: Circuit breaker configuration
    println!("\n2. Building circuit breaker configuration:");
    let cb_config = DynamicConfigBuilder::new()
        .circuit_breaker()
        .failure_threshold(5)
        .reset_timeout(Duration::from_secs(30))
        .half_open_max_operations(2)
        .done()?
        .build();

    println!(
        "   Failure threshold: {:?}",
        cb_config.get_value("circuit_breaker.failure_threshold")?
    );
    println!(
        "   Reset timeout: {:?}",
        cb_config.get_value("circuit_breaker.reset_timeout_ms")?
    );
    println!(
        "   Half-open max ops: {:?}",
        cb_config.get_value("circuit_breaker.half_open_max_operations")?
    );

    // Example 3: Bulkhead configuration
    println!("\n3. Building bulkhead configuration:");
    let bulkhead_config = DynamicConfigBuilder::new()
        .bulkhead()
        .max_concurrency(10)
        .queue_size(20)
        .timeout(Duration::from_secs(5))
        .done()?
        .build();

    println!(
        "   Max concurrency: {:?}",
        bulkhead_config.get_value("bulkhead.max_concurrency")?
    );
    println!(
        "   Queue size: {:?}",
        bulkhead_config.get_value("bulkhead.queue_size")?
    );
    println!(
        "   Timeout: {:?}",
        bulkhead_config.get_value("bulkhead.timeout_ms")?
    );

    // Example 4: Complete configuration with multiple patterns
    println!("\n4. Building complete configuration with all patterns:");
    let complete_config = DynamicConfigBuilder::new()
        .retry()
        .max_attempts(3)
        .base_delay(Duration::from_millis(100))
        .done()?
        .circuit_breaker()
        .failure_threshold(5)
        .reset_timeout(Duration::from_secs(30))
        .half_open_max_operations(2)
        .done()?
        .bulkhead()
        .max_concurrency(10)
        .queue_size(20)
        .timeout(Duration::from_secs(5))
        .done()?
        .build();

    println!("   Configuration successfully built with:");
    println!(
        "   - Retry: {} attempts, {}ms base delay",
        complete_config.get_value("retry.max_attempts")?,
        complete_config.get_value("retry.base_delay_ms")?
    );
    println!(
        "   - Circuit Breaker: {} failures, {}ms reset timeout",
        complete_config.get_value("circuit_breaker.failure_threshold")?,
        complete_config.get_value("circuit_breaker.reset_timeout_ms")?
    );
    println!(
        "   - Bulkhead: {} concurrent, {} queue size",
        complete_config.get_value("bulkhead.max_concurrency")?,
        complete_config.get_value("bulkhead.queue_size")?
    );

    // Example 5: Type safety demonstration (compile-time checks)
    println!("\n5. Type safety demonstration:");
    println!("   The builder API enforces type safety at compile time:");
    println!("   - max_attempts(usize) - only accepts integers");
    println!("   - base_delay(Duration) - only accepts Duration");
    println!("   - timeout(Duration) - only accepts Duration");
    println!("   Trying to pass wrong types results in compile errors!");

    // Example 6: Validation demonstration
    println!("\n6. Validation demonstration:");
    let validation_result = DynamicConfigBuilder::new()
        .retry()
        .max_attempts(0) // Invalid: must be > 0
        .base_delay(Duration::from_millis(100))
        .done();

    match validation_result {
        Ok(_) => println!("   Unexpected success"),
        Err(e) => println!("   Validation error (expected): {}", e),
    }

    // Example 7: Missing required field
    println!("\n7. Missing required field demonstration:");
    let missing_field_result = DynamicConfigBuilder::new()
        .retry()
        .max_attempts(3)
        // Missing: base_delay()
        .done();

    match missing_field_result {
        Ok(_) => println!("   Unexpected success"),
        Err(e) => println!("   Required field error (expected): {}", e),
    }

    println!("\n=== Example completed successfully! ===");
    Ok(())
}
