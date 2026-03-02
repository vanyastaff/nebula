# CoreError

`CoreError` is a 26-variant enum covering every failure category that can arise at the
foundation layer. All higher-level crates define their own error types; `CoreError` is
used only in `nebula-core` itself and as a conversion target for `std` errors.

---

## Variants

### Client Errors (4xx — not retryable)

| Variant | Fields | Error code |
|---|---|---|
| `Validation` | `message`, `field?`, `value?` | `VALIDATION_ERROR` |
| `NotFound` | `resource_type`, `resource_id` | `NOT_FOUND_ERROR` |
| `AlreadyExists` | `resource_type`, `resource_id` | `ALREADY_EXISTS_ERROR` |
| `PermissionDenied` | `operation`, `resource`, `reason?` | `PERMISSION_DENIED_ERROR` |
| `Authentication` | `reason`, `user_id?` | `AUTHENTICATION_ERROR` |
| `Authorization` | `operation`, `resource`, `user_id?`, `tenant_id?` | `AUTHORIZATION_ERROR` |
| `InvalidInput` | `message`, `field?`, `value?` | `INVALID_INPUT_ERROR` |
| `RateLimitExceeded` | `limit`, `period`, `retry_after?` | `RATE_LIMIT_ERROR` |
| `ResourceExhausted` | `resource`, `limit`, `current?` | `RESOURCE_EXHAUSTED_ERROR` |
| `InvalidState` | `current_state`, `expected_state?`, `operation` | `INVALID_STATE_ERROR` |

### Server Errors (5xx — may be retryable)

| Variant | Fields | Retryable? |
|---|---|---|
| `Timeout` | `operation`, `duration` | Yes |
| `ServiceUnavailable` | `service`, `reason`, `retry_after?` | Yes |
| `Network` | `operation`, `reason`, `retryable` | If `retryable = true` |
| `Storage` | `operation`, `reason`, `backend?` | Yes |
| `NodeExecution` | `node_id`, `execution_id?`, `reason`, `retryable` | If `retryable = true` |
| `Internal` | `message`, `code?` | No |
| `Configuration` | `message`, `file?`, `line?` | No |
| `Dependency` | `dependency`, `reason`, `operation?` | No |
| `Serialization` | `message`, `format?` | No |
| `Deserialization` | `message`, `format?`, `data?` | No |

### Domain Errors

| Variant | Fields |
|---|---|
| `WorkflowExecution` | `workflow_id`, `execution_id?`, `node_id?`, `reason` |
| `ExpressionEvaluation` | `expression`, `reason`, `context?` |
| `ResourceManagement` | `operation`, `resource_type`, `reason` |
| `Cluster` | `operation`, `reason`, `node_id?` |
| `Tenant` | `tenant_id`, `reason`, `operation?` |

---

## Classification Methods

```rust
err.is_retryable()    // Timeout, RateLimit, ServiceUnavailable, Network{retryable:true}, Storage, NodeExecution{retryable:true}
err.is_client_error() // Validation, NotFound, AlreadyExists, PermissionDenied, Auth*, InvalidInput, RateLimit, ResourceExhausted, InvalidState
err.is_server_error() // Internal, ServiceUnavailable, Configuration, Dependency, Network, Storage, Cluster
err.error_code()      // &'static str — e.g. "VALIDATION_ERROR"
err.user_message()    // String — human-readable, safe to show in UI
```

---

## Constructors

```rust
CoreError::validation("email is required")
CoreError::validation_with_details("too short", "password", "abc")
CoreError::not_found("User", user_id.to_string())
CoreError::already_exists("Workflow", wf_id.to_string())
CoreError::permission_denied("delete", "credential", None::<String>)
CoreError::authentication("invalid token", Some(user_id))
CoreError::authorization("write", "workflow", Some(user_id), Some(tenant_id))
CoreError::invalid_input("port must be 1-65535")
CoreError::timeout("database query", Duration::from_secs(5))
CoreError::rate_limit_exceeded(100, Duration::from_secs(60), None)
CoreError::internal("unexpected panic in executor")
CoreError::service_unavailable("redis", "connection refused", Some(Duration::from_secs(5)))
```

---

## CoreResult\<T\>

```rust
pub type CoreResult<T> = Result<T, CoreError>;
```

---

## From Conversions

| Source type | Becomes |
|---|---|
| `std::io::Error` | `CoreError::Internal` |
| `serde_json::Error` | `CoreError::Serialization` |
| `postcard::Error` | `CoreError::Serialization` |
| `uuid::Error` | `CoreError::InvalidInput` |
| `chrono::ParseError` | `CoreError::InvalidInput` |

---

## Design Note

`CoreError` is intentionally broad — it covers concepts from networking to tenancy
because `nebula-core` traits return it (e.g. `Serializable`). Higher-level crates
do **not** use `CoreError` for their own domain errors; they define their own error
enums using `thiserror`. `CoreError` is the vocabulary of the foundation layer only.
