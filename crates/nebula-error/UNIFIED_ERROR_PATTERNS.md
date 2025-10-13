# Unified Error Patterns for Nebula Crates

This document describes the unified error handling patterns that all Nebula crates should follow for consistency, performance, and maintainability.

## Overview

The Nebula ecosystem uses a centralized error handling approach where:
1. `nebula-error` provides the core `NebulaError` type and utilities
2. Individual crates define domain-specific error kinds within the centralized system
3. All errors eventually convert to `NebulaError` for consistent handling

## Performance Optimizations Applied

### Memory Layout
- `Cow<'static, str>` for messages (zero-alloc for static strings)
- `&'static str` for error codes (zero-alloc)
- `Box<ErrorContext>` for lazy allocation
- Optimized field ordering by size and alignment

### Zero-Cost Abstractions
- Static dispatch for error conversion
- Inline methods for hot paths
- Compile-time error code validation
- Macro-based error creation for common patterns

## Recommended Patterns

### Pattern 1: Pure Nebula Integration (Recommended)

For new crates or simple error handling needs:

```rust
// src/error.rs
use nebula_error::prelude::*;

// Re-export nebula-error types for convenience
pub use nebula_error::{NebulaError, Result};

// Domain-specific error constructors
impl NebulaError {
    /// Create a configuration validation error
    pub fn config_validation(field: &str, reason: &str) -> Self {
        validation_error!("Configuration field '{}' is invalid: {}", field, reason)
    }
    
    /// Create a service connection error
    pub fn service_connection(service: &str, details: &str) -> Self {
        service_unavailable_error!("{} service connection failed: {}", service, details)
    }
}

// Example usage
pub fn validate_config(config: &Config) -> Result<()> {
    ensure!(
        !config.name.is_empty(), 
        NebulaError::config_validation("name", "cannot be empty")
    );
    Ok(())
}
```

### Pattern 2: Wrapper with Domain Context

For crates that need additional domain-specific context:

```rust
// src/error.rs
use nebula_error::{NebulaError, Result as NebulaResult};

#[derive(Debug, Clone)]
pub struct DomainError {
    /// The underlying nebula error
    inner: NebulaError,
    /// Domain-specific context
    domain_context: Option<DomainContext>,
}

#[derive(Debug, Clone)]
pub struct DomainContext {
    pub operation: String,
    pub entity_id: Option<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

impl DomainError {
    /// Create from NebulaError
    pub fn from_nebula(error: NebulaError) -> Self {
        Self {
            inner: error,
            domain_context: None,
        }
    }
    
    /// Add domain context
    pub fn with_domain_context(mut self, context: DomainContext) -> Self {
        self.domain_context = Some(context);
        self
    }
    
    /// Get the underlying NebulaError
    pub fn inner(&self) -> &NebulaError {
        &self.inner
    }
}

// Conversion traits
impl From<NebulaError> for DomainError {
    fn from(error: NebulaError) -> Self {
        Self::from_nebula(error)
    }
}

impl From<DomainError> for NebulaError {
    fn from(error: DomainError) -> Self {
        match error.domain_context {
            Some(context) => {
                let mut nebula_error = error.inner;
                nebula_error = nebula_error.with_details(&format!(
                    "Domain operation: {}", context.operation
                ));
                nebula_error
            }
            None => error.inner,
        }
    }
}

pub type Result<T> = std::result::Result<T, DomainError>;
```

### Pattern 3: Migration from Existing Error Types

For existing crates that need to gradually migrate:

```rust
// src/error.rs - Migration approach
use nebula_error::{NebulaError, ErrorKind};
use thiserror::Error;

// Keep existing error enum for backward compatibility
#[derive(Error, Debug, Clone)]
pub enum LegacyError {
    #[error("Validation error: {message}")]
    Validation { message: String },
    
    #[error("Not found: {resource}")]
    NotFound { resource: String },
    
    // ... other variants
}

// Add conversion to NebulaError
impl From<LegacyError> for NebulaError {
    fn from(error: LegacyError) -> Self {
        match error {
            LegacyError::Validation { message } => {
                Self::validation(message)
            }
            LegacyError::NotFound { resource } => {
                Self::not_found("resource", resource)
            }
        }
    }
}

// Provide both old and new APIs
pub type Result<T> = std::result::Result<T, LegacyError>;
pub type NebulaResult<T> = std::result::Result<T, NebulaError>;

// Migration helper
pub trait IntoNebulaResult<T> {
    fn into_nebula_result(self) -> NebulaResult<T>;
}

impl<T> IntoNebulaResult<T> for Result<T> {
    fn into_nebula_result(self) -> NebulaResult<T> {
        self.map_err(|e| e.into())
    }
}
```

## Performance Best Practices

### 1. Use Static Strings Where Possible

```rust
// ✅ Good - Zero allocation
let error = validation_error!("Invalid input");

// ✅ Good - Static string optimization
let error = NebulaError::new_static(
    ErrorKind::Client(ClientError::Validation {
        message: "Invalid input".into(),
    }),
    "Invalid input"
);

// ❌ Avoid - Unnecessary allocation
let error = validation_error!("Invalid {}", "input");  // Only when dynamic
```

### 2. Leverage Macros for Common Patterns

```rust
// ✅ Use macros for better ergonomics and performance
let error = not_found_error!("User", user_id);
let error = timeout_error!("database_query", Duration::from_secs(30));
let error = memory_error!("pool_exhausted", "connection_pool", 100);

// ✅ Use ensure! for validation
ensure!(age >= 18, validation_error!("Age must be at least 18"));
ensure!(name.len() <= 100, validation_error!("Name too long"));
```

### 3. Optimize Error Chains

```rust
// ✅ Use error chaining traits
database_operation()
    .map_err(|e| e.chain_with("Failed to update user record"))
    .map_err(|e| e.chain_retryable())  // Mark as retryable if needed
```

## Testing Patterns

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_creation_and_properties() {
        let error = validation_error!("Invalid email");
        
        assert_eq!(error.error_code(), "VALIDATION_ERROR");
        assert!(!error.is_retryable());
        assert!(error.is_client_error());
        assert!(error.user_message().contains("Invalid email"));
    }
    
    #[test]
    fn test_error_conversion() {
        let io_error = std::io::Error::new(
            std::io::ErrorKind::NotFound, 
            "File not found"
        );
        let nebula_error: NebulaError = io_error.into();
        
        assert!(nebula_error.is_client_error());
        assert!(!nebula_error.is_retryable());
    }
    
    #[test]
    fn test_error_with_context() {
        let error = internal_error!("Database connection failed")
            .with_context(
                ErrorContext::new("Processing user request")
                    .with_user_id("user123")
                    .with_request_id("req456")
            );
        
        assert!(error.context().is_some());
        let context = error.context().unwrap();
        assert_eq!(context.description, "Processing user request");
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_retry_with_error_handling() {
    use nebula_error::{RetryStrategy, retry};
    
    let strategy = RetryStrategy::default()
        .with_max_attempts(3)
        .with_base_delay(Duration::from_millis(10));
    
    let mut attempts = 0;
    let result = retry(|| async {
        attempts += 1;
        if attempts < 3 {
            Err(service_unavailable_error!("database", "connection pool full"))
        } else {
            Ok("success")
        }
    }, &strategy).await;
    
    assert!(result.is_ok());
    assert_eq!(attempts, 3);
}
```

## Documentation Standards

### Error Module Documentation

```rust
//! # Error Handling for [Crate Name]
//!
//! This module provides error handling using the unified Nebula error system.
//! All errors are based on [`NebulaError`] for consistent handling across
//! the Nebula ecosystem.
//!
//! ## Quick Start
//!
//! ```rust
//! use crate_name::{Result, Error};
//!
//! fn example_operation() -> Result<String> {
//!     ensure!(condition, validation_error!("Condition not met"));
//!     Ok("success".to_string())
//! }
//! ```
//!
//! ## Error Categories
//!
//! - **Validation Errors**: Input validation failures
//! - **Configuration Errors**: Invalid configuration
//! - **Service Errors**: External service failures
//! - **Resource Errors**: Resource exhaustion or unavailability
```

### Error Constructor Documentation

```rust
impl NebulaError {
    /// Create a database connection error
    ///
    /// This error indicates a failure to connect to the database.
    /// It is marked as retryable with a default retry delay.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let error = NebulaError::database_connection("Connection timeout");
    /// assert!(error.is_retryable());
    /// ```
    pub fn database_connection(reason: impl Into<String>) -> Self {
        service_unavailable_error!("database", reason.into())
            .with_retry_info(true, Some(Duration::from_secs(2)))
    }
}
```

## Migration Checklist

When updating existing crates to use this unified pattern:

- [ ] Replace custom error enums with NebulaError integration
- [ ] Add performance-optimized constructors using macros
- [ ] Update error conversion implementations
- [ ] Add proper error classification (client/server/system)
- [ ] Implement retry logic where appropriate
- [ ] Update documentation with examples
- [ ] Add comprehensive test coverage
- [ ] Benchmark error creation performance
- [ ] Validate error serialization/deserialization
- [ ] Update public API documentation

## Common Pitfalls to Avoid

1. **Over-allocation**: Don't use `format!` for static error messages
2. **Missing classification**: Always ensure errors are properly classified
3. **Inconsistent retry behavior**: Use standard retry patterns
4. **Poor error context**: Add meaningful context for debugging
5. **Breaking API changes**: Provide backward compatibility during migration

## Performance Metrics

Target performance characteristics:
- Error creation: <100ns for static errors
- Error cloning: <50ns for simple errors
- Error serialization: <1μs for typical errors
- Memory footprint: <128 bytes per error instance
- Error code lookup: 0ns (compile-time constants)

## Examples

See the `examples/` directory in `nebula-error` for complete examples of:
- Simple domain-specific errors
- Complex error handling with retry logic
- Performance benchmarking
- Error serialization for network transfer
- Integration with existing error systems
