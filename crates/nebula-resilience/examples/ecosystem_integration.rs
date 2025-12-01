//! Example demonstrating nebula-resilience integration with the Nebula ecosystem

use std::time::Duration;
use tokio::time::sleep;

use nebula_resilience::prelude::*;
use nebula_resilience::{log_result, timeout_fn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with nebula-log
    nebula_log::auto_init()?;

    info!("Starting resilience ecosystem integration example");

    // Example 1: Database operations with const generic circuit breaker
    info!("=== Database Operations Example ===");

    // Type-safe circuit breaker: 5 failures threshold, 60 second reset
    let circuit_breaker = CircuitBreaker::<5, 60_000>::with_defaults()?;

    let db_result = circuit_breaker
        .execute(|| async {
            info!(operation = "database_query", "Executing database query");

            // Simulate database operation
            sleep(Duration::from_millis(100)).await;

            // Simulate occasional failures for demonstration
            if rand::random::<f64>() > 0.7 {
                return Err(ResilienceError::Custom {
                    message: "Database connection timeout".to_string(),
                    retryable: true,
                    source: None,
                });
            }

            Ok("Database result".to_string())
        })
        .await;

    log_result!(db_result, "database_query", "Database operation");

    // Example 2: HTTP API with const generic retry
    info!("=== HTTP API Example ===");

    // Type-safe retry: 3 max attempts with exponential backoff
    let retry_strategy = exponential_retry::<3>()?;

    let api_result = retry_strategy
        .execute_resilient(|| async {
            info!(
                operation = "api_call",
                endpoint = "/users",
                "Making HTTP API call"
            );

            // Simulate HTTP call
            sleep(Duration::from_millis(200)).await;

            // Simulate network issues
            if rand::random::<f64>() > 0.8 {
                return Err(ResilienceError::Timeout {
                    duration: Duration::from_secs(10),
                    context: Some("HTTP request timeout".to_string()),
                });
            }

            Ok(serde_json::json!({
                "users": [
                    {"id": 1, "name": "Alice"},
                    {"id": 2, "name": "Bob"}
                ]
            }))
        })
        .await;

    log_result!(api_result, "api_call", "HTTP API operation");

    // Example 3: File operations with timeout
    info!("=== File Operations Example ===");

    // Simulate file operations with timeout
    for i in 1..=3u64 {
        let file_result = timeout_fn(Duration::from_secs(5), async move {
            info!(file = %format!("file_{}.txt", i), "Processing file");

            // Simulate file processing
            sleep(Duration::from_millis(50 * i)).await;

            Ok::<_, ResilienceError>(format!("Processed file_{}.txt", i))
        })
        .await;

        match file_result {
            Ok(Ok(result)) => info!(result = %result, "File processed successfully"),
            Ok(Err(err)) => warn!(error = %err, "File processing failed"),
            Err(_) => warn!(file = %format!("file_{}.txt", i), "File processing timed out"),
        }
    }

    // Example 4: Using bulkhead for concurrency control
    info!("=== Bulkhead Example ===");

    let bulkhead = Bulkhead::with_config(BulkheadConfig {
        max_concurrency: 5,
        queue_size: 10,
        timeout: Some(Duration::from_secs(2)),
    });

    let cache_result = bulkhead
        .execute(|| async {
            info!(
                operation = "cache_get",
                key = "user:123",
                "Getting from cache"
            );
            sleep(Duration::from_millis(10)).await;
            Ok("cached_value".to_string())
        })
        .await?;

    info!(cached_value = %cache_result, "Retrieved from cache");

    info!("Resilience ecosystem integration example completed successfully");
    Ok(())
}

// Mock rand function for the example
mod rand {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    pub fn random<T>() -> f64
    where
        T: 'static,
    {
        let mut hasher = DefaultHasher::new();
        SystemTime::now().hash(&mut hasher);
        std::any::TypeId::of::<T>().hash(&mut hasher);
        let hash = hasher.finish();
        (hash % 1000) as f64 / 1000.0
    }
}
