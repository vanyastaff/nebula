# Nebula Resilience

Production-ready resilience patterns for Rust applications, fully integrated with the Nebula ecosystem.

## Overview

`nebula-resilience` provides comprehensive resilience patterns including circuit breakers, retries, timeouts, bulkheads, rate limiting, and more. It's designed to work seamlessly with other Nebula crates for configuration management, logging, and value handling.

## Key Features

### 🛡️ Resilience Patterns
- **Circuit Breaker**: Automatic failure detection and recovery
- **Retry**: Configurable retry strategies with backoff
- **Timeout**: Operation timeouts with context
- **Bulkhead**: Resource isolation and concurrency control
- **Rate Limiting**: Multiple rate limiting algorithms
- **Fallback**: Graceful degradation strategies
- **Hedge**: Reduce tail latency with parallel requests

### 🔧 Ecosystem Integration
- **Configuration**: Built on `nebula-config` for flexible configuration management
- **Logging**: Structured logging with `nebula-log`
- **Dynamic Values**: Runtime configuration using `nebula-value`

### 📊 Advanced Features
- **Policy Composition**: Combine multiple patterns
- **Hot Configuration Reload**: Update settings without restarts
- **Metrics Collection**: Built-in observability
- **Async/Await Support**: First-class async support

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-resilience = "0.1.0"
nebula-log = "0.1.0"  # For logging
tokio = { version = "1.0", features = ["full"] }
```

## Basic Usage

```rust
use nebula_resilience::prelude::*;

#[tokio::main]
async fn main() -> ConfigResult<()> {
    // Initialize logging
    nebula_log::auto_init()?;

    // Create a resilience policy
    let policy = ResiliencePolicy::named("my-service")
        .with_timeout(Duration::from_secs(10))
        .with_retry(RetryStrategy::exponential(3, Duration::from_millis(100)))
        .with_circuit_breaker(CircuitBreakerConfig::default());

    // Execute operations with resilience
    let result = policy.execute(|| async {
        // Your operation here
        Ok("Success!")
    }).await?;

    info!(result = %result, "Operation completed");
    Ok(())
}
```

## Best Practices

1. **Use Presets**: Start with predefined configurations for common scenarios
2. **Structured Logging**: Always initialize `nebula-log` for observability
3. **Hot Reload**: Use `DynamicConfig` for runtime configuration updates
4. **Error Classification**: Leverage automatic error classification for retry decisions
5. **Policy Composition**: Combine patterns for comprehensive resilience
6. **Metrics**: Enable metrics collection for monitoring and alerting

## Examples

See the `examples/` directory for complete examples:

- `ecosystem_integration.rs` - Full ecosystem integration demo

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

Resilience patterns for the Nebula workflow engine, providing robust error handling, retry mechanisms, circuit breakers, and bulkhead patterns.

## Features

- **Timeout Management**: Configurable timeouts for all I/O operations
- **Retry Strategies**: Exponential backoff with jitter and circuit breaker integration
- **Circuit Breakers**: Automatic failure detection and recovery
- **Bulkheads**: Resource isolation and parallelism limits
- **Resilience Policies**: Configurable resilience strategies per operation type

## Quick Start

```rust
use nebula_resilience::{
    timeout, retry, circuit_breaker, bulkhead,
    ResiliencePolicy, ResilienceBuilder
};

// Simple timeout wrapper
let result = timeout(Duration::from_secs(30), async_operation()).await;

// With retry and circuit breaker
let policy = ResiliencePolicy::default()
    .with_timeout(Duration::from_secs(10))
    .with_retry(3, Duration::from_secs(1))
    .with_circuit_breaker(5, Duration::from_secs(60));

let result = policy.execute(async_operation()).await;

// Bulkhead for resource isolation
let bulkhead = bulkhead::Bulkhead::new(10);
let result = bulkhead.execute(async_operation()).await;
```

## Core Components

### Timeout

The timeout module provides wrappers for async operations with configurable timeouts:

```rust
use nebula_resilience::timeout;
use std::time::Duration;

// Basic timeout
let result = timeout(Duration::from_secs(30), async_operation()).await;

// Create timeout-aware future without executing
let timeout_future = timeout::with_timeout(Duration::from_secs(10), async_operation());
```

### Retry

Retry strategies with exponential backoff and jitter:

```rust
use nebula_resilience::retry::{RetryStrategy, retry};

let strategy = RetryStrategy::new(3, Duration::from_secs(1))
    .with_max_delay(Duration::from_secs(30))
    .with_jitter(0.1);

let result = retry(strategy, || async_operation()).await;
```

### Circuit Breaker

Automatic failure detection and recovery:

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};

let config = CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(60),
    half_open_max_operations: 3,
    count_timeouts: true,
};

let circuit_breaker = CircuitBreaker::with_config(config);
let result = circuit_breaker.execute(async_operation()).await;
```

### Bulkhead

Resource isolation and parallelism limits:

```rust
use nebula_resilience::bulkhead::Bulkhead;

let bulkhead = Bulkhead::new(10); // Max 10 concurrent operations
let result = bulkhead.execute(async_operation()).await;

// With timeout
let result = bulkhead.execute_with_timeout(
    Duration::from_secs(30),
    async_operation()
).await;
```

### Resilience Policy

Unified policy combining multiple resilience patterns:

```rust
use nebula_resilience::{ResiliencePolicy, ResilienceBuilder};

let policy = ResilienceBuilder::new()
    .timeout(Duration::from_secs(10))
    .retry(3, Duration::from_secs(1))
    .circuit_breaker(5, Duration::from_secs(60))
    .bulkhead(20)
    .build();

let result = policy.execute(async_operation()).await;
```

## Predefined Policies

Common resilience policies for different operation types:

```rust
use nebula_resilience::policies;

// Database operations
let db_policy = policies::database();

// HTTP API calls
let http_policy = policies::http_api();

// File operations
let file_policy = policies::file_operations();

// Long-running operations
let long_policy = policies::long_running();

// Critical operations (minimal resilience)
let critical_policy = policies::critical();
```

## Error Handling

All resilience operations return `ResilienceResult<T>` which can contain:

- `ResilienceError::Timeout` - Operation exceeded timeout
- `ResilienceError::CircuitBreakerOpen` - Circuit breaker is open
- `ResilienceError::BulkheadFull` - Bulkhead is at capacity
- `ResilienceError::RetryLimitExceeded` - All retry attempts failed
- `ResilienceError::Cancelled` - Operation was cancelled
- `ResilienceError::InvalidConfig` - Invalid configuration

```rust
use nebula_resilience::ResilienceError;

match result {
    Ok(value) => println!("Success: {:?}", value),
    Err(ResilienceError::Timeout { duration }) => {
        println!("Operation timed out after {:?}", duration);
    }
    Err(ResilienceError::CircuitBreakerOpen { state }) => {
        println!("Circuit breaker is open: {}", state);
    }
    Err(e) => println!("Other error: {:?}", e),
}
```

## Configuration

### Retry Strategy

```rust
let strategy = RetryStrategy::new(3, Duration::from_secs(1))
    .with_max_delay(Duration::from_secs(30))
    .with_jitter(0.1)
    .without_exponential();
```

### Circuit Breaker

```rust
let config = CircuitBreakerConfig {
    failure_threshold: 5,           // Open after 5 failures
    reset_timeout: Duration::from_secs(60),  // Wait 60s before half-open
    half_open_max_operations: 3,    // Allow 3 operations in half-open
    count_timeouts: true,           // Count timeouts as failures
};
```

### Bulkhead

```rust
let bulkhead = BulkheadBuilder::new(10)
    .with_max_queue_size(50)
    .with_reject_when_full(true)
    .build();
```

## Best Practices

1. **Always use timeouts** for I/O operations to prevent indefinite blocking
2. **Configure retry strategies** based on error characteristics (retryable vs terminal)
3. **Use circuit breakers** for external dependencies that can fail repeatedly
4. **Apply bulkheads** to limit resource consumption and prevent cascading failures
5. **Combine patterns** using resilience policies for comprehensive protection
6. **Monitor and adjust** resilience parameters based on production metrics

## Examples

### Database Operation with Full Resilience

```rust
use nebula_resilience::policies;

async fn save_user(user: User) -> Result<(), Error> {
    let policy = policies::database();
    
    policy.execute(|| async {
        // Database operation here
        db.save_user(&user).await
    }).await.map_err(|e| Error::Resilience(e))?
}
```

### HTTP API Call with Custom Policy

```rust
use nebula_resilience::{ResilienceBuilder, ResiliencePolicy};

async fn call_external_api() -> Result<ApiResponse, Error> {
    let policy = ResilienceBuilder::new()
        .timeout(Duration::from_secs(15))
        .retry(2, Duration::from_secs(2))
        .circuit_breaker(3, Duration::from_secs(30))
        .bulkhead(25)
        .build();
    
    policy.execute(|| async {
        // HTTP request here
        http_client.get("/api/data").await
    }).await.map_err(|e| Error::Resilience(e))?
}
```

### File Processing with Bulkhead

```rust
use nebula_resilience::bulkhead::Bulkhead;

async fn process_files(files: Vec<PathBuf>) -> Result<Vec<ProcessedFile>, Error> {
    let bulkhead = Bulkhead::new(5); // Max 5 concurrent file operations
    
    let mut handles = Vec::new();
    for file in files {
        let bulkhead = bulkhead.clone();
        let handle = tokio::spawn(async move {
            bulkhead.execute(|| async {
                // File processing logic here
                process_single_file(&file).await
            }).await
        });
        handles.push(handle);
    }
    
    // Collect results
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await??;
        results.push(result);
    }
    
    Ok(results)
}
```

## Testing

The crate includes comprehensive tests for all resilience patterns:

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_circuit_breaker_failure_threshold
```

## Contributing

When contributing to the resilience crate:

1. Follow Rust naming conventions
2. Add comprehensive tests for new functionality
3. Use structured logging with tracing
4. Ensure all async operations handle cancellation properly
5. Add documentation for public APIs
6. Follow the established error handling patterns

## License

This crate is part of the Nebula workflow engine and is licensed under the same terms.
