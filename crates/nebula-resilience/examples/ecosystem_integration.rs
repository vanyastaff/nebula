//! Example demonstrating nebula-resilience integration with the Nebula ecosystem

use std::time::Duration;
use tokio::time::sleep;

use nebula_resilience::prelude::*;
use nebula_resilience::{log_result, retry};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with nebula-log
    nebula_log::auto_init()?;

    info!("Starting resilience ecosystem integration example");

    // Use predefined configurations for common scenarios
    let db_config = ResiliencePresets::database();
    let http_config = ResiliencePresets::http_api();

    info!(
        preset = "database",
        config = ?db_config.to_flat_map(),
        "Using database preset configuration"
    );

    // Create a dynamic configuration for a custom service
    let mut custom_config = DynamicConfig::new();
    custom_config.set_value("circuit_breaker.failure_threshold", Value::integer(10))?;
    custom_config.set_value("retry.max_attempts", Value::integer(5))?;
    custom_config.set_value("timeout.duration", Value::text("15s"))?;

    info!(
        service = "custom",
        config = ?custom_config.to_flat_map(),
        "Created custom dynamic configuration"
    );

    // Example 1: Database operations with preset configuration
    info!("=== Database Operations Example ===");

    let circuit_breaker = CircuitBreaker::with_config(CircuitBreakerConfig {
        failure_threshold: 5,
        reset_timeout: Duration::from_secs(60),
        half_open_max_operations: 3,
        count_timeouts: true,
    });

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

    // Example 2: HTTP API with retry
    info!("=== HTTP API Example ===");

    let retry_strategy = RetryStrategy::exponential_backoff(3, Duration::from_millis(500));
    let api_result = retry(retry_strategy, || async {
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

    // Example 3: File operations with light resilience
    info!("=== File Operations Example ===");

    let file_config = ResiliencePresets::file_io();
    let retry_config = file_config.get_value("retry.max_attempts")?;

    info!(
        operation = "file_operations",
        retry_attempts = %retry_config,
        "Performing file operations with light resilience"
    );

    // Simulate file operations
    for i in 1..=3 {
        let file_result = timeout(Duration::from_secs(5), async {
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

    // Example 5: Configuration hot-reloading demonstration
    info!("=== Configuration Hot-Reload Example ===");

    let mut dynamic_config = DynamicConfig::new();

    // Initial configuration
    dynamic_config.set_value("feature.enabled", Value::boolean(true))?;
    dynamic_config.set_value("feature.timeout", Value::text("5s"))?;

    info!(
        config = ?dynamic_config.to_flat_map(),
        "Initial dynamic configuration"
    );

    // Simulate configuration update
    dynamic_config.set_value("feature.timeout", Value::text("10s"))?;
    dynamic_config.set_value("feature.retry_count", Value::integer(3))?;

    info!(
        config = ?dynamic_config.to_flat_map(),
        "Updated dynamic configuration"
    );

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
