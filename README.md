# Nebula Project

## Overview

Nebula is a Rust project consisting of multiple crates that provide functionality for logging and parameter management. This README provides an overview of the main components and examples of how to use them.

## Crates

### nebula-log

A comprehensive logging library built on top of the `tracing` crate. It provides structured logging with context tracking, span management, and various output formats.

Key features:
- Structured logging with JSON or pretty format
- Execution context tracking
- Span management for tracing execution flow
- Timer utilities for performance measurement
- Platform-specific logging adapters

### nebula-parameter

A flexible parameter management system that provides type-safe access to parameters with validation and metadata.

Key features:
- Type-safe parameter access
- Parameter validation
- Metadata for parameters (descriptions, constraints, etc.)
- Error handling for parameter operations

## Examples

### nebula-log Example

```rust
use nebula_log::{Logger, ExecutionContext};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the logger with development settings
    Logger::init_dev()?;
    
    // Basic logging examples
    tracing::info!("Starting the application");
    tracing::warn!(user_id = "user-123", "User session is about to expire");
    tracing::error!(error_code = 500, "Database connection failed");
    
    // Using execution context
    let span = tracing::span!(tracing::Level::INFO, "process_request");
    let _guard = span.enter();
    
    // Set execution context information
    ExecutionContext::set_execution_context(
        "exec-123", 
        "workflow-456", 
        "action-789", 
        Some("account-abc"), 
        Some("user-def"), 
        Some("corr-ghi")
    );
    
    // Log within the span
    tracing::info!("Processing user request");
    
    // Using a timer
    {
        let _timer = nebula_log::Timer::new("database_query");
        thread::sleep(Duration::from_millis(50)); // Simulate database query
    }
    
    Ok(())
}
```

### nebula-parameter Example

```rust
use nebula_parameter::core::{
    ParameterKey, ParameterProvider, TypedParameterProvider,
    ParameterMetadata, ParameterTypeInfo, ParameterValue, ParameterError
};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create parameter keys
    let user_id = ParameterKey::try_new("user_id").unwrap();
    let user_name = ParameterKey::try_new("user_name").unwrap();
    
    // Create a parameter collection
    let mut params = HashMap::<ParameterKey, Box<dyn ParameterValue>>::new();
    
    // Add parameters with different types
    params.insert(user_id.clone(), Box::new("user-123".to_string()));
    params.insert(user_name.clone(), Box::new("John Doe".to_string()));
    
    // Create a simple parameter provider
    struct SimpleParameterProvider {
        params: HashMap<ParameterKey, Box<dyn ParameterValue>>
    }
    
    impl ParameterProvider for SimpleParameterProvider {
        fn get(&self, key: &ParameterKey) -> Result<Box<dyn ParameterValue>, ParameterError> {
            self.params.get(key)
                .map(|v| v.clone_value())
                .ok_or_else(|| ParameterError::ParameterNotFound { key: key.clone() })
        }
        
        fn contains(&self, key: &ParameterKey) -> bool {
            self.params.contains_key(key)
        }
        
        fn keys(&self) -> Vec<ParameterKey> {
            self.params.keys().cloned().collect()
        }
    }
    
    let provider = SimpleParameterProvider { params };
    
    // Get typed parameters
    let user_id_val: String = provider.get_typed(&user_id)?;
    let user_name_val: String = provider.get_typed(&user_name)?;
    
    println!("User ID: {}", user_id_val);
    println!("User Name: {}", user_name_val);
    
    Ok(())
}
```

## Running the Examples

To run the examples:

```
# Run the nebula-log example
cd crates/nebula-log
cargo run --example basic_logging

# Run the nebula-parameter example
cd crates/nebula-parameter
cargo run --example parameter_usage
```

## Running Tests

To run the tests:

```
# Run nebula-log tests
cd crates/nebula-log
cargo test

# Run nebula-parameter tests
cd crates/nebula-parameter
cargo test
```