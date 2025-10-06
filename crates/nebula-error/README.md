# nebula-error

Centralized error handling system for the Nebula workflow engine.

## Overview

`nebula-error` provides a unified error type system with proper error classification, context propagation, and retry logic for workflow orchestration. It helps distinguish between client errors (bad input), server errors (transient failures), and system errors.

## Key Features

- **Unified Error Type** - Single `NebulaError` with structured error kinds
- **Error Classification** - Automatic categorization (client/server/system/workflow)
- **Rich Context** - Structured error context with metadata and correlation IDs
- **Retry Logic** - Built-in retry strategies with exponential backoff and jitter
- **Workflow Support** - Specialized errors for nodes, triggers, connectors
- **Seamless Conversion** - Auto-conversion from standard library errors

## Quick Start

```rust
use nebula_error::{NebulaError, Result, ResultExt};

fn process_data() -> Result<String> {
    // Validation errors (client errors - not retryable)
    if invalid_input {
        return Err(NebulaError::validation("Invalid data format"));
    }

    // Add context to errors
    let result = risky_operation()
        .context("Processing user data")?;

    Ok(result)
}
```

## Error Categories

### Client Errors (4xx) - Not Retryable

```rust
use nebula_error::NebulaError;

// Validation
let err = NebulaError::validation("Missing required field");

// Not found
let err = NebulaError::not_found("User", "123");

// Permission denied
let err = NebulaError::permission_denied("read", "sensitive_data");

// Invalid input
let err = NebulaError::invalid_input("email", "not-an-email");
```

### Server Errors (5xx) - Often Retryable

```rust
use nebula_error::NebulaError;
use std::time::Duration;

// Service unavailable
let err = NebulaError::service_unavailable("database", "connection pool exhausted");

// Timeout
let err = NebulaError::timeout("API call", Duration::from_secs(30));

// Internal error
let err = NebulaError::internal("Unexpected state");

// Check if retryable
assert!(err.is_retryable());
```

### Workflow-Specific Errors

```rust
use nebula_error::NebulaError;

// Workflow errors
let err = NebulaError::workflow_not_found("user-onboarding");
let err = NebulaError::node_execution_failed("send-email", "SMTP timeout");
let err = NebulaError::trigger_invalid_cron_expression("* * * * * *", "invalid");

// Connector errors
let err = NebulaError::connector_api_call_failed("slack", "/api/chat", 500);
let err = NebulaError::credential_not_found("slack-oauth-token");

// Execution limits
let err = NebulaError::execution_memory_limit_exceeded(512, 256);
let err = NebulaError::execution_queue_full(1000, 1000);
```

## Retry Strategies

```rust
use nebula_error::{RetryStrategy, retry};
use std::time::Duration;

async fn example() -> nebula_error::Result<()> {
    let strategy = RetryStrategy::default()
        .with_max_attempts(3)
        .with_base_delay(Duration::from_millis(100))
        .with_max_delay(Duration::from_secs(5))
        .with_jitter(true);

    let result = retry(|| async {
        // Your async operation here
        call_external_api().await
    }, &strategy).await?;

    Ok(())
}
```

## Error Context

```rust
use nebula_error::{NebulaError, ErrorContext};

let context = ErrorContext::new()
    .with_metadata("user_id", "123")
    .with_metadata("request_id", "abc-xyz")
    .with_correlation_id("trace-123");

let err = NebulaError::internal("Database error")
    .with_context(context);
```

## Using the Prelude

```rust
use nebula_error::prelude::*;

// All error types are now available
fn my_function() -> Result<()> {
    // ...
    Ok(())
}
```

## Architecture

```
nebula-error/
├── core/           # Core error types and NebulaError
├── kinds/          # Error kind classification
├── context/        # Error context and metadata
├── retry/          # Retry strategies and logic
└── conversion/     # External error conversions
```

## Error Kind Hierarchy

```
ErrorKind
├── Client (4xx)
│   ├── Validation
│   ├── NotFound
│   ├── PermissionDenied
│   └── InvalidInput
├── Server (5xx)
│   ├── Internal
│   ├── ServiceUnavailable
│   └── Timeout
├── System
│   ├── IoError
│   ├── SerializationError
│   └── ConfigurationError
└── Workflow
    ├── WorkflowNotFound
    ├── NodeExecutionFailed
    ├── TriggerError
    └── ConnectorError
```

## Best Practices

1. **Use specific error constructors** instead of generic ones
2. **Add context** to errors as they propagate up the call stack
3. **Check `is_retryable()`** before implementing retry logic
4. **Use workflow-specific errors** for domain errors
5. **Include metadata** for debugging (request IDs, user IDs, etc.)

## License

Licensed under the same terms as the Nebula project.
