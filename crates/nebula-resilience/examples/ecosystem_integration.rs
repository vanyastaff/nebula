//! Example demonstrating nebula-resilience integration with the Nebula ecosystem

use std::time::Duration;
use tokio::time::sleep;

use nebula_resilience::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with nebula-log
    nebula_log::auto_init()?;

    info!("Starting resilience ecosystem integration example");

    // Create a resilience configuration manager
    let mut config_manager = ResilienceConfigManager::new().await?;

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
    custom_config.set_value("circuit_breaker.failure_threshold", Value::from(10))?;
    custom_config.set_value("retry.max_attempts", Value::from(5))?;
    custom_config.set_value("timeout.duration", Value::from("15s"))?;

    info!(
        service = "custom",
        config = ?custom_config.to_flat_map(),
        "Created custom dynamic configuration"
    );

    // Example 1: Database operations with preset configuration
    info!("=== Database Operations Example ===");

    let db_circuit_config = db_config.get_config::<CircuitBreakerConfig>("circuit_breaker")?;
    let circuit_breaker = CircuitBreaker::with_config(db_circuit_config);

    let db_result = circuit_breaker.execute(|| async {
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
    }).await;

    match db_result {
        Ok(result) => info!(result = %result, "Database operation succeeded"),
        Err(err) => warn!(error = %err, "Database operation failed"),
    }

    // Example 2: HTTP API with comprehensive resilience
    info!("=== HTTP API Example ===");

    let policy = ResiliencePolicy::named("http-api")
        .with_timeout(Duration::from_secs(10))
        .with_retry(RetryStrategy::exponential(3, Duration::from_millis(500)))
        .with_circuit_breaker(CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_secs(30),
            half_open_max_operations: 2,
            count_timeouts: true,
        });

    let api_result = policy.execute(|| async {
        info!(operation = "api_call", endpoint = "/users", "Making HTTP API call");

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
    }).await;

    match api_result {
        Ok(result) => info!(result = %result, "API call succeeded"),
        Err(err) => warn!(error = %err, "API call failed"),
    }

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

            Ok(format!("Processed file_{}.txt", i))
        }).await;

        match file_result {
            Ok(Ok(result)) => info!(result = %result, "File processed successfully"),
            Ok(Err(err)) => warn!(error = %err, "File processing failed"),
            Err(_) => warn!(file = %format!("file_{}.txt", i), "File processing timed out"),
        }
    }

    // Example 4: Using resilience manager for complex scenarios
    info!("=== Resilience Manager Example ===");

    let manager = ResilienceManager::builder()
        .with_policy("database",
            ResiliencePolicy::named("database")
                .with_timeout(Duration::from_secs(5))
                .with_circuit_breaker(CircuitBreakerConfig::default())
        )
        .with_policy("cache",
            ResiliencePolicy::named("cache")
                .with_timeout(Duration::from_millis(100))
                .with_retry(RetryStrategy::fixed(2, Duration::from_millis(10)))
        )
        .build();

    // Use different policies for different operations
    let cache_result = manager.execute("cache", || async {
        info!(operation = "cache_get", key = "user:123", "Getting from cache");
        sleep(Duration::from_millis(10)).await;
        Ok("cached_value".to_string())
    }).await?;

    info!(cached_value = %cache_result, "Retrieved from cache");

    // Example 5: Configuration hot-reloading demonstration
    info!("=== Configuration Hot-Reload Example ===");

    let mut dynamic_config = DynamicConfig::new();

    // Initial configuration
    dynamic_config.set_value("feature.enabled", Value::from(true))?;
    dynamic_config.set_value("feature.timeout", Value::from("5s"))?;

    info!(
        config = ?dynamic_config.to_flat_map(),
        "Initial dynamic configuration"
    );

    // Simulate configuration update
    dynamic_config.set_value("feature.timeout", Value::from("10s"))?;
    dynamic_config.set_value("feature.retry_count", Value::from(3))?;

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