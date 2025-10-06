# nebula-error Documentation

## For AI Agents

Structured information about the `nebula-error` crate for AI agents and automated tools.

## Crate Purpose

`nebula-error` is the **centralized error handling system** for Nebula. It provides:
1. Unified error type (`NebulaError`) with classification
2. Retry logic with exponential backoff
3. Rich error context with metadata
4. Automatic error conversion from stdlib and third-party types

## Module Structure

```
nebula-error/
├── src/
│   ├── lib.rs          # Main exports and prelude
│   ├── core/
│   │   ├── mod.rs      # Core error type
│   │   ├── result.rs   # Result type and extensions
│   │   └── context.rs  # Error context
│   ├── kinds/
│   │   ├── mod.rs      # Error kind enum
│   │   ├── client.rs   # Client errors (4xx)
│   │   ├── server.rs   # Server errors (5xx)
│   │   ├── system.rs   # System errors
│   │   └── workflow.rs # Workflow errors
│   ├── retry/
│   │   ├── mod.rs      # Retry logic
│   │   └── strategy.rs # Retry strategies
│   └── conversion/     # External error conversions
└── Cargo.toml
```

## Core Types

### NebulaError

```rust
pub struct NebulaError {
    kind: ErrorKind,
    message: String,
    context: Option<ErrorContext>,
    source: Option<Box<dyn Error + Send + Sync>>,
}
```

**Methods**:
- `kind(&self) -> &ErrorKind` - Get error classification
- `is_retryable(&self) -> bool` - Check if error can be retried
- `with_context(self, ErrorContext) -> Self` - Add context
- `context(&self) -> Option<&ErrorContext>` - Get context

### ErrorKind

```rust
pub enum ErrorKind {
    // Client errors (4xx) - user's fault, not retryable
    Validation,
    NotFound,
    PermissionDenied,
    InvalidInput,
    RateLimited,

    // Server errors (5xx) - server's fault, often retryable
    Internal,
    ServiceUnavailable,
    Timeout,
    DatabaseError,

    // System errors - infrastructure issues
    IoError,
    SerializationError,
    ConfigurationError,
    NetworkError,

    // Workflow-specific errors
    WorkflowNotFound,
    NodeExecutionFailed,
    TriggerError,
    ConnectorError,
    CredentialError,
    ExecutionLimitExceeded,
}
```

### RetryStrategy

```rust
pub struct RetryStrategy {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
    jitter: bool,
}
```

**Builder pattern**:
```rust
RetryStrategy::default()
    .with_max_attempts(3)
    .with_base_delay(Duration::from_millis(100))
    .with_max_delay(Duration::from_secs(5))
    .with_jitter(true)
```

## Error Classification

### Client Errors (4xx)
- **NOT retryable** - user must fix input
- Examples: validation, not found, permission denied
- HTTP equivalent: 400-499

### Server Errors (5xx)
- **Often retryable** - transient failures
- Examples: internal, service unavailable, timeout
- HTTP equivalent: 500-599

### System Errors
- **Sometimes retryable** - depends on cause
- Examples: I/O errors, network errors
- Infrastructure-level issues

### Workflow Errors
- **Domain-specific** - depends on context
- Examples: workflow not found, node failed
- Application-level issues

## Common Constructors

### Validation Errors
```rust
NebulaError::validation("message")
NebulaError::invalid_input("field", "value")
NebulaError::required_field("field_name")
```

### Not Found Errors
```rust
NebulaError::not_found("Entity", "id")
NebulaError::workflow_not_found("workflow-id")
NebulaError::credential_not_found("cred-id")
```

### Server Errors
```rust
NebulaError::internal("message")
NebulaError::service_unavailable("service", "reason")
NebulaError::timeout("operation", duration)
NebulaError::database_error("query failed")
```

### Workflow Errors
```rust
NebulaError::node_execution_failed("node-id", "reason")
NebulaError::trigger_invalid_cron_expression("expr", "error")
NebulaError::connector_api_call_failed("connector", "endpoint", status)
NebulaError::execution_memory_limit_exceeded(used, limit)
```

## Retry Logic

### Basic Retry
```rust
use nebula_error::retry;

let result = retry(|| async {
    call_api().await
}, &RetryStrategy::default()).await?;
```

### Retry with Timeout
```rust
use nebula_error::retry_with_timeout;
use std::time::Duration;

let result = retry_with_timeout(
    || async { call_api().await },
    &RetryStrategy::default(),
    Duration::from_secs(30)
).await?;
```

### Custom Retry Logic
```rust
for attempt in 1..=3 {
    match operation().await {
        Ok(result) => return Ok(result),
        Err(e) if e.is_retryable() && attempt < 3 => {
            tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
            continue;
        }
        Err(e) => return Err(e),
    }
}
```

## Error Context

### Adding Context
```rust
let context = ErrorContext::new()
    .with_metadata("user_id", "123")
    .with_metadata("workflow_id", "abc")
    .with_correlation_id("trace-xyz");

let error = NebulaError::internal("Failed")
    .with_context(context);
```

### Using ResultExt
```rust
use nebula_error::ResultExt;

fn process() -> Result<()> {
    risky_operation()
        .context("Processing user data")?;
    Ok(())
}
```

## Error Conversion

Automatic conversion from common types:

```rust
// From std::io::Error
let io_err: std::io::Error = /* ... */;
let nebula_err: NebulaError = io_err.into();

// From serde_json::Error
let json_err: serde_json::Error = /* ... */;
let nebula_err: NebulaError = json_err.into();

// From anyhow::Error
let anyhow_err: anyhow::Error = /* ... */;
let nebula_err: NebulaError = anyhow_err.into();
```

## Integration Points

### Used By
- All Nebula crates for error handling
- Workflow execution engine
- API endpoints
- Background workers

### Uses
- `thiserror` - Error derive macros
- `anyhow` - Error context
- `tokio` - Async retry logic

## When to Use

Use `nebula-error` when you need:
1. ✅ Unified error handling across Nebula
2. ✅ Error classification (client/server/system)
3. ✅ Automatic retry logic
4. ✅ Rich error context
5. ✅ Workflow-specific errors

## When NOT to Use

❌ Don't use `nebula-error` for:
- Simple boolean checks (use `Option` or `bool`)
- Performance-critical paths where errors are common
- Pure validation logic (consider returning `Vec<ValidationError>`)

## Testing

```bash
# Run tests
cargo test -p nebula-error

# Run with all features
cargo test -p nebula-error --all-features
```

## Performance Considerations

- Error creation is fast (stack-allocated)
- Context is optional (no overhead if not used)
- Retry logic uses tokio's sleep (efficient)
- Error conversion is zero-cost

## Thread Safety

- `NebulaError` is `Send + Sync`
- Safe to share across threads
- Retry logic is async-safe

## Best Practices

1. **Use specific constructors** - `validation()` instead of `new(ErrorKind::Validation)`
2. **Add context as you propagate** - Use `.context()` at each layer
3. **Check retryability** - Don't retry client errors
4. **Include metadata** - Add user IDs, request IDs for debugging
5. **Use workflow errors** - For domain-specific failures
6. **Propagate with `?`** - Automatic conversion works

## Examples

### Basic Error Handling
```rust
use nebula_error::{NebulaError, Result};

fn divide(a: i32, b: i32) -> Result<i32> {
    if b == 0 {
        return Err(NebulaError::validation("Division by zero"));
    }
    Ok(a / b)
}
```

### With Context
```rust
use nebula_error::{Result, ResultExt};

async fn process_user(user_id: &str) -> Result<()> {
    fetch_user(user_id)
        .await
        .context("Fetching user from database")?;

    update_user(user_id)
        .await
        .context("Updating user preferences")?;

    Ok(())
}
```

### With Retry
```rust
use nebula_error::{retry, RetryStrategy, Result};
use std::time::Duration;

async fn call_api_with_retry() -> Result<String> {
    let strategy = RetryStrategy::default()
        .with_max_attempts(3)
        .with_base_delay(Duration::from_millis(100));

    retry(|| async {
        call_external_api().await
    }, &strategy).await
}
```

## Version

Current version: See [Cargo.toml](./Cargo.toml)
