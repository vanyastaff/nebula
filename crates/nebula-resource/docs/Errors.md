---
title: Errors Resources
tags: [nebula, nebula-resource, docs, errors]
status: draft
created: 2025-08-17
---

# Error Types Reference

## Overview

Nebula Resource uses a comprehensive error system based on Rust's `Result` type and the `thiserror` crate for ergonomic error handling.

## Core Error Type

### `Error`

The main error enum for the resource system.

````rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // Resource errors
    #[error("Resource not found: {id}

### Error Logging

```rust
use tracing::{error, warn, info};

impl Error {
    /// Log error with appropriate level
    pub fn log(&self) {
        match self.severity() {
            ErrorSeverity::Fatal => {
                error!(
                    error = %self,
                    error_code = %self.error_code(),
                    "Fatal error occurred"
                );
            }
            ErrorSeverity::Error => {
                error!(
                    error = %self,
                    error_code = %self.error_code(),
                    "Error occurred"
                );
            }
            ErrorSeverity::Warning => {
                warn!(
                    error = %self,
                    error_code = %self.error_code(),
                    "Warning occurred"
                );
            }
            ErrorSeverity::Info => {
                info!(
                    error = %self,
                    error_code = %self.error_code(),
                    "Info level error"
                );
            }
        }
    }
    
    /// Get error severity
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Error::Internal(_) | Error::DeadlockDetected { .. } => ErrorSeverity::Fatal,
            Error::ResourceNotFound { .. } | Error::ValidationFailed { .. } => ErrorSeverity::Warning,
            Error::AcquisitionTimeout { .. } => ErrorSeverity::Info,
            _ => ErrorSeverity::Error,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorSeverity {
    Fatal,
    Error,
    Warning,
    Info,
}
````

## Custom Error Types

### Creating Domain-Specific Errors

```rust
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Connection pool exhausted")]
    PoolExhausted,
    
    #[error("Query timeout: {query}")]
    QueryTimeout { query: String },
    
    #[error("Transaction rollback: {reason}")]
    TransactionRollback { reason: String },
    
    #[error("Constraint violation: {constraint}")]
    ConstraintViolation { constraint: String },
}

// Convert to main error type
impl From<DatabaseError> for Error {
    fn from(err: DatabaseError) -> Self {
        Error::External {
            source: "database".to_string(),
            error: err.to_string(),
        }
    }
}
```

### Extending Error Type

```rust
pub trait ErrorExt {
    /// Get error code for API responses
    fn error_code(&self) -> String;
    
    /// Get additional error details
    fn details(&self) -> Option<HashMap<String, Value>>;
    
    /// Check if error should trigger circuit breaker
    fn triggers_circuit_breaker(&self) -> bool;
    
    /// Get retry strategy for this error
    fn retry_strategy(&self) -> RetryStrategy;
}

impl ErrorExt for Error {
    fn error_code(&self) -> String {
        match self {
            Error::ResourceNotFound { .. } => "RESOURCE_NOT_FOUND".to_string(),
            Error::ResourceAlreadyExists { .. } => "RESOURCE_EXISTS".to_string(),
            Error::ValidationFailed { .. } => "VALIDATION_FAILED".to_string(),
            Error::AuthenticationFailed { .. } => "AUTH_FAILED".to_string(),
            Error::AuthorizationFailed { .. } => "AUTHZ_FAILED".to_string(),
            _ => "INTERNAL_ERROR".to_string(),
        }
    }
    
    fn details(&self) -> Option<HashMap<String, Value>> {
        match self {
            Error::ValidationFailed { field, reason } => {
                Some(HashMap::from([
                    ("field".to_string(), json!(field)),
                    ("reason".to_string(), json!(reason)),
                ]))
            }
            Error::ConnectionTimeout { endpoint, duration } => {
                Some(HashMap::from([
                    ("endpoint".to_string(), json!(endpoint)),
                    ("timeout_seconds".to_string(), json!(duration.as_secs())),
                ]))
            }
            _ => None,
        }
    }
    
    fn triggers_circuit_breaker(&self) -> bool {
        matches!(self,
            Error::ConnectionFailed { .. } |
            Error::ConnectionTimeout { .. } |
            Error::HealthCheckFailed { .. }
        )
    }
    
    fn retry_strategy(&self) -> RetryStrategy {
        if self.is_retryable() {
            RetryStrategy::ExponentialBackoff {
                initial_delay: Duration::from_millis(100),
                max_delay: Duration::from_secs(30),
                max_attempts: 3,
            }
        } else {
            RetryStrategy::None
        }
    }
}
```

## Error Boundaries

### Panic Handling

```rust
use std::panic;

pub struct ErrorBoundary;

impl ErrorBoundary {
    /// Catch panics and convert to errors
    pub async fn catch_panic<F, T>(f: F) -> Result<T, Error>
    where
        F: FnOnce() -> T + panic::UnwindSafe,
    {
        match panic::catch_unwind(f) {
            Ok(result) => Ok(result),
            Err(panic_info) => {
                let message = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                
                Err(Error::Internal(format!("Panic: {}", message)))
            }
        }
    }
}
```

### Error Propagation Control

```rust
pub struct ErrorPropagation;

impl ErrorPropagation {
    /// Stop error propagation at boundary
    pub fn boundary<T>(result: Result<T, Error>) -> Result<Option<T>, Error> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.is_fatal() => Err(e),
            Err(e) => {
                e.log();
                Ok(None)
            }
        }
    }
    
    /// Transform error for external API
    pub fn external<T>(result: Result<T, Error>) -> Result<T, ExternalError> {
        result.map_err(|e| ExternalError {
            message: self.sanitize_message(&e),
            code: e.error_code(),
            status: e.to_http_status(),
        })
    }
    
    fn sanitize_message(&self, error: &Error) -> String {
        // Remove sensitive information
        match error {
            Error::ConnectionFailed { .. } => "Connection failed".to_string(),
            Error::AuthenticationFailed { .. } => "Authentication failed".to_string(),
            _ => error.to_string(),
        }
    }
}
```

## Testing Errors

### Error Assertions

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_conversion() {
        let io_error = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "File not found"
        );
        
        let error: Error = io_error.into();
        assert!(matches!(error, Error::FileNotFound { .. }));
    }
    
    #[test]
    fn test_error_retryable() {
        let error = Error::ConnectionTimeout {
            endpoint: "localhost:5432".to_string(),
            duration: Duration::from_secs(30),
        };
        
        assert!(error.is_retryable());
        assert!(!error.is_fatal());
    }
    
    #[tokio::test]
    async fn test_error_recovery() {
        let mut attempts = 0;
        
        let result = with_retry(
            || async {
                attempts += 1;
                if attempts < 3 {
                    Err(Error::Network("Temporary failure".into()))
                } else {
                    Ok("Success")
                }
            },
            5
        ).await;
        
        assert_eq!(result.unwrap(), "Success");
        assert_eq!(attempts, 3);
    }
}
```

### Error Injection

```rust
#[cfg(test)]
pub struct ErrorInjector {
    error_rate: f32,
    error_type: Error,
}

impl ErrorInjector {
    pub fn new(error_rate: f32, error_type: Error) -> Self {
        Self { error_rate, error_type }
    }
    
    pub fn maybe_inject(&self) -> Result<(), Error> {
        if rand::random::<f32>() < self.error_rate {
            Err(self.error_type.clone())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_with_error_injection() {
        let injector = ErrorInjector::new(
            0.5,
            Error::Network("Injected error".into())
        );
        
        let mut successes = 0;
        let mut failures = 0;
        
        for _ in 0..100 {
            match injector.maybe_inject() {
                Ok(_) => successes += 1,
                Err(_) => failures += 1,
            }
        }
        
        // Should be roughly 50/50
        assert!(failures > 30 && failures < 70);
    }
}
```

## Error Metrics

### Tracking Error Rates

```rust
use prometheus::{IntCounterVec, HistogramVec};

pub struct ErrorMetrics {
    error_counter: IntCounterVec,
    error_duration: HistogramVec,
}

impl ErrorMetrics {
    pub fn new() -> Self {
        let error_counter = IntCounterVec::new(
            Opts::new("resource_errors_total", "Total number of errors"),
            &["error_type", "resource_type", "severity"]
        ).unwrap();
        
        let error_duration = HistogramVec::new(
            HistogramOpts::new("error_recovery_duration", "Time to recover from error"),
            &["error_type"]
        ).unwrap();
        
        Self {
            error_counter,
            error_duration,
        }
    }
    
    pub fn record_error(&self, error: &Error, resource_type: &str) {
        self.error_counter
            .with_label_values(&[
                &error.error_code(),
                resource_type,
                &error.severity().to_string(),
            ])
            .inc();
    }
    
    pub fn record_recovery(&self, error: &Error, duration: Duration) {
        self.error_duration
            .with_label_values(&[&error.error_code()])
            .observe(duration.as_secs_f64());
    }
}
```

## Best Practices

1. **Use specific error types** - Create domain-specific errors when appropriate
    
2. **Add context** - Include relevant information in error messages
    
3. **Make errors actionable** - Tell users what they can do
    
4. **Log appropriately** - Use correct log levels
    
5. **Handle all errors** - Don't ignore error results
    
6. **Test error paths** - Write tests for error conditions
    
7. **Document errors** - Include possible errors in API documentation
    
8. **Sanitize for external APIs** - Don't leak implementation details
    
9. **Track error metrics** - Monitor error rates and types
    
10. **Design for recovery** - Make errors recoverable when possible")] ResourceNotFound { id: ResourceId },
    
    #[error("Resource already exists: {id}")] ResourceAlreadyExists { id: ResourceId },
    
    #[error("Resource type mismatch: expected {expected}, got {actual}")] ResourceTypeMismatch { expected: String, actual: String, },
    
    #[error("Resource in use: {id} (users: {count})")] ResourceInUse { id: ResourceId, count: usize },
    
    #[error("Resource initialization failed: {resource}: {reason}")] InitializationFailed { resource: ResourceId, reason: String, },
    
    // Pool errors #[error("Pool exhausted: no resources available")] PoolExhausted,
    
    #[error("Pool acquisition timeout after {duration:?}")] AcquisitionTimeout { duration: Duration },
    
    #[error("Pool validation failed: {reason}")] PoolValidationFailed { reason: String },
    
    #[error("Pool scaling failed: {reason}")] PoolScalingFailed { reason: String },
    
    // Health check errors #[error("Health check failed: {resource}: {reason}")] HealthCheckFailed { resource: ResourceId, reason: String, },
    
    #[error("Health check timeout after {duration:?}")] HealthCheckTimeout { duration: Duration },
    
    // Configuration errors #[error("Invalid configuration: {field}: {reason}")] InvalidConfiguration { field: String, reason: String, },
    
    #[error("Missing required configuration: {field}")] MissingConfiguration { field: String },
    
    #[error("Configuration validation failed: {errors:?}")] ConfigurationValidationFailed { errors: Vec<String> },
    
    // Lifecycle errors #[error("Invalid lifecycle transition: {from:?} -> {to:?}")] InvalidLifecycleTransition { from: LifecycleState, to: LifecycleState, },
    
    #[error("Lifecycle hook failed: {hook}: {reason}")] LifecycleHookFailed { hook: String, reason: String, },
    
    // Migration errors #[error("Migration failed from {from} to {to}: {reason}")] MigrationFailed { from: Version, to: Version, reason: String, },
    
    #[error("No migration path found from {from} to {to}")] NoMigrationPath { from: Version, to: Version, },
    
    #[error("Migration validation failed for version {version}: {error}")] MigrationValidationFailed { version: Version, error: String, },
    
    // Scope errors #[error("Scope not found: {scope:?}")] ScopeNotFound { scope: ResourceScope },
    
    #[error("Access denied: resource {resource} from scope {from_scope:?} to {resource_scope:?}")] AccessDenied { resource: ResourceId, from_scope: ResourceScope, resource_scope: ResourceScope, },
    
    #[error("Scope already exists: {scope:?}")] ScopeAlreadyExists { scope: ResourceScope },
    
    // Quarantine errors #[error("Resource quarantined: {resource}: {reason:?}")] ResourceQuarantined { resource: ResourceId, reason: QuarantineReason, },
    
    #[error("Recovery failed: {resource}: {reason}")] RecoveryFailed { resource: ResourceId, reason: String, },
    
    // Network errors #[error("Connection failed: {endpoint}: {reason}")] ConnectionFailed { endpoint: String, reason: String, },
    
    #[error("Connection timeout: {endpoint} after {duration:?}")] ConnectionTimeout { endpoint: String, duration: Duration, },
    
    #[error("Network error: {0}")] Network(String),
    
    // IO errors #[error("IO error: {0}")] Io(#[from] std::io::Error),
    
    #[error("File not found: {path}")] FileNotFound { path: PathBuf },
    
    #[error("Permission denied: {path}")] PermissionDenied { path: PathBuf },
    
    // Serialization errors #[error("Serialization error: {0}")] Serialization(#[from] serde_json::Error),
    
    #[error("Deserialization error: {type_name}: {error}")] Deserialization { type_name: String, error: String, },
    
    // Validation errors #[error("Validation failed: {field}: {reason}")] ValidationFailed { field: String, reason: String, },
    
    #[error("Multiple validation errors: {0:?}")] MultipleValidationErrors(Vec<ValidationError>),
    
    // Security errors #[error("Authentication failed: {reason}")] AuthenticationFailed { reason: String },
    
    #[error("Authorization failed: {action} on {resource}")] AuthorizationFailed { action: String, resource: ResourceId, },
    
    #[error("Encryption error: {0}")] Encryption(String),
    
    // Timeout errors #[error("Operation timeout: {operation} after {duration:?}")] OperationTimeout { operation: String, duration: Duration, },
    
    // Concurrency errors #[error("Lock acquisition failed: {resource}")] LockAcquisitionFailed { resource: ResourceId },
    
    #[error("Deadlock detected: resources {resources:?}")] DeadlockDetected { resources: Vec<ResourceId> },
    
    // Other errors #[error("Internal error: {0}")] Internal(String),
    
    #[error("Not implemented: {feature}")] NotImplemented { feature: String },
    
    #[error("Unsupported operation: {operation}")] UnsupportedOperation { operation: String },
    
    #[error("External error: {source}: {error}")] External { source: String, error: String, },
    
    #[error(transparent)] Other(#[from] anyhow::Error), }
    

````

## Error Categories

### Resource Errors

Errors related to resource management:

```rust
impl Error {
    /// Check if error is related to resources
    pub fn is_resource_error(&self) -> bool {
        matches!(self,
            Error::ResourceNotFound { .. } |
            Error::ResourceAlreadyExists { .. } |
            Error::ResourceTypeMismatch { .. } |
            Error::ResourceInUse { .. } |
            Error::InitializationFailed { .. }
        )
    }
    
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self,
            Error::ConnectionTimeout { .. } |
            Error::AcquisitionTimeout { .. } |
            Error::LockAcquisitionFailed { .. } |
            Error::Network(_)
        )
    }
    
    /// Check if error is fatal
    pub fn is_fatal(&self) -> bool {
        matches!(self,
            Error::InvalidConfiguration { .. } |
            Error::MissingConfiguration { .. } |
            Error::NotImplemented { .. } |
            Error::Internal(_)
        )
    }
}
````

### Validation Errors

Detailed validation error type:

```rust
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub value: Option<String>,
    pub constraint: ValidationConstraint,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum ValidationConstraint {
    Required,
    MinLength(usize),
    MaxLength(usize),
    Pattern(String),
    Range { min: f64, max: f64 },
    Enum(Vec<String>),
    Custom(String),
}
```

## Error Context

### Adding Context

```rust
use nebula_resource::error::{Error, ErrorContext};

// Add context to errors
let result = database.connect()
    .await
    .context("Failed to connect to database")?;

// With structured context
let result = database.connect()
    .await
    .with_context(|| ErrorContext {
        operation: "database_connect".to_string(),
        resource: resource_id.clone(),
        details: HashMap::from([
            ("host", config.host.clone()),
            ("port", config.port.to_string()),
        ]),
    })?;
```

### Error Context Structure

```rust
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub operation: String,
    pub resource: ResourceId,
    pub details: HashMap<String, String>,
    pub timestamp: Instant,
    pub trace_id: Option<String>,
}
```

## Error Handling Patterns

### Basic Error Handling

```rust
use nebula_resource::error::Error;

async fn handle_resource(id: ResourceId) -> Result<(), Error> {
    let resource = manager.get_resource(&id)
        .await
        .map_err(|e| match e {
            Error::ResourceNotFound { .. } => {
                // Handle missing resource
                Error::Internal("Resource required but not found".into())
            }
            other => other,
        })?;
    
    resource.process().await?;
    Ok(())
}
```

### Error Recovery

```rust
async fn with_retry<T, F>(
    mut operation: F,
    max_retries: u32,
) -> Result<T, Error>
where
    F: FnMut() -> Future<Output = Result<T, Error>>,
{
    let mut last_error = None;
    
    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if e.is_retryable() => {
                last_error = Some(e);
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
            }
            Err(e) => return Err(e),
        }
    }
    
    Err(last_error.unwrap_or_else(|| Error::Internal("Max retries exceeded".into())))
}
```

### Error Aggregation

```rust
#[derive(Debug)]
pub struct ErrorCollector {
    errors: Vec<Error>,
}

impl ErrorCollector {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }
    
    pub fn add(&mut self, error: Error) {
        self.errors.push(error);
    }
    
    pub fn add_result<T>(&mut self, result: Result<T, Error>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.add(e);
                None
            }
        }
    }
    
    pub fn into_result(self) -> Result<(), Error> {
        if self.errors.is_empty() {
            Ok(())
        } else if self.errors.len() == 1 {
            Err(self.errors.into_iter().next().unwrap())
        } else {
            Err(Error::Multiple(self.errors))
        }
    }
}
```

## Error Conversion

### From Standard Errors

```rust
impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Error::FileNotFound {
                path: PathBuf::from("unknown"),
            },
            std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
                path: PathBuf::from("unknown"),
            },
            _ => Error::Io(err),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Serialization(err)
    }
}
```

### To HTTP Status Codes

```rust
impl Error {
    pub fn to_http_status(&self) -> u16 {
        match self {
            Error::ResourceNotFound { .. } => 404,
            Error::ResourceAlreadyExists { .. } => 409,
            Error::ValidationFailed { .. } => 400,
            Error::AuthenticationFailed { .. } => 401,
            Error::AuthorizationFailed { .. } => 403,
            Error::OperationTimeout { .. } => 408,
            Error::Internal(_) => 500,
            Error::NotImplemented { .. } => 501,
            _ => 500,
        }
    }
}
```

## Error Reporting

### Structured Error Response

```rust
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorInfo,
    pub request_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    pub details: Option<HashMap<String, Value>>,
    pub trace: Option<Vec<ErrorTrace>>,
}

#[derive(Debug, Serialize)]
pub struct ErrorTrace {
    pub file: String,
    pub line: u32,
    pub function: String,
}

impl From<Error> for ErrorResponse {
    fn from(error: Error) -> Self {
        ErrorResponse {
            error: ErrorInfo {
                code: error.error_code(),
                message: error.to_string(),
                details: error.details(),
                trace: error.backtrace(),
            },
            request_id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
        }
    }
}
```