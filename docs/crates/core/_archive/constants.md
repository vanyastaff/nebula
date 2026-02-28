# Constants Reference

`nebula-core::constants` defines system-wide defaults and limits. All values are
`const` — zero runtime cost. Grouped by subsystem.

---

## System Identity

```rust
SYSTEM_NAME        = "Nebula"
SYSTEM_VERSION     = env!("CARGO_PKG_VERSION")  // from Cargo.toml at compile time
SYSTEM_DESCRIPTION = "High-performance workflow engine"
```

---

## Timeouts

| Constant | Value | Applies to |
|---|---|---|
| `DEFAULT_TIMEOUT` | 30 s | General operations |
| `DEFAULT_DATABASE_TIMEOUT` | 5 s | DB queries |
| `DEFAULT_HTTP_TIMEOUT` | 10 s | HTTP requests |
| `DEFAULT_GRPC_TIMEOUT` | 15 s | gRPC calls |
| `DEFAULT_API_TIMEOUT` | 30 s | API handler |
| `DEFAULT_SESSION_TIMEOUT` | 1 h | User sessions |

---

## Retry Settings

```rust
DEFAULT_MAX_RETRIES      = 3
DEFAULT_RETRY_DELAY      = 1 s
DEFAULT_MAX_RETRY_DELAY  = 60 s
```

---

## Resilience Settings

```rust
// Circuit breaker
DEFAULT_CIRCUIT_BREAKER_FAILURE_THRESHOLD = 5
DEFAULT_CIRCUIT_BREAKER_RESET_TIMEOUT     = 60 s

// Bulkhead
DEFAULT_BULKHEAD_MAX_CONCURRENT  = 10
DEFAULT_BULKHEAD_MAX_QUEUE_SIZE  = 100
```

---

## Workflow & Node Limits

```rust
DEFAULT_MAX_WORKFLOW_NODES      = 1_000
DEFAULT_MAX_WORKFLOW_DEPTH      = 50
DEFAULT_MAX_EXECUTION_TIME      = 3_600 s  // 1 hour
DEFAULT_MAX_NODE_INPUT_SIZE     = 1 MB
DEFAULT_MAX_NODE_OUTPUT_SIZE    = 1 MB
DEFAULT_MAX_NODE_EXECUTION_TIME = 300 s    // 5 minutes
```

---

## Action & Expression Limits

```rust
DEFAULT_MAX_ACTION_PARAMETERS       = 100
DEFAULT_MAX_ACTION_RESULT_SIZE      = 10 MB
DEFAULT_MAX_EXPRESSION_LENGTH       = 10_000
DEFAULT_MAX_EXPRESSION_DEPTH        = 100
DEFAULT_MAX_EXPRESSION_EXECUTION_TIME = 10 s
```

---

## Memory & Cache

```rust
DEFAULT_MAX_MEMORY_MB   = 1_024  // 1 GB
DEFAULT_CACHE_TTL       = 300 s  // 5 minutes
DEFAULT_MAX_CACHE_SIZE  = 10_000
```

---

## Storage

```rust
DEFAULT_MAX_STORAGE_KEY_LENGTH  = 255
DEFAULT_MAX_STORAGE_VALUE_SIZE  = 100 MB
DEFAULT_STORAGE_BATCH_SIZE      = 1_000
```

---

## Multi-Tenancy

```rust
DEFAULT_MAX_TENANTS                 = 1_000
DEFAULT_MAX_WORKFLOWS_PER_TENANT    = 10_000
DEFAULT_MAX_EXECUTIONS_PER_TENANT   = 100_000
```

---

## Clustering

```rust
DEFAULT_CLUSTER_HEARTBEAT_INTERVAL = 5 s
DEFAULT_CLUSTER_ELECTION_TIMEOUT   = 100 s
DEFAULT_CLUSTER_MAX_NODES          = 100
```

---

## Validation Limits

```rust
DEFAULT_MAX_STRING_LENGTH      = 10_000
DEFAULT_MAX_ARRAY_SIZE         = 10_000
DEFAULT_MAX_OBJECT_PROPERTIES  = 1_000

limits::MAX_WORKFLOW_NAME_LENGTH      = 255
limits::MAX_WORKFLOW_DESCRIPTION_LENGTH = 1_000
limits::MAX_NODE_NAME_LENGTH          = 255
limits::MAX_ACTION_NAME_LENGTH        = 255
limits::MAX_PARAMETER_NAME_LENGTH     = 255
limits::MAX_PARAMETER_VALUE_LENGTH    = 10_000
limits::MAX_TAG_LENGTH                = 100
limits::MAX_TAGS_PER_ENTITY           = 50
limits::MAX_METADATA_KEYS             = 100
limits::MAX_METADATA_VALUE_LENGTH     = 1_000
```

---

## Security Constants

```rust
DEFAULT_MIN_PASSWORD_LENGTH = 8
DEFAULT_MAX_PASSWORD_LENGTH = 128
DEFAULT_MAX_LOGIN_ATTEMPTS  = 5

security::MIN_PASSWORD_ENTROPY  = 3.0
security::MAX_SESSION_DURATION  = 30 days
security::MIN_SESSION_DURATION  = 5 min
security::MAX_API_KEY_LENGTH    = 64
security::MIN_API_KEY_LENGTH    = 16
```

---

## Validation Patterns

```rust
patterns::IDENTIFIER_PATTERN = r"^[a-zA-Z_][a-zA-Z0-9_-]*$"
patterns::EMAIL_PATTERN      = r"^[^@]+@[^@]+\.[^@]+$"
patterns::URL_PATTERN        = r"^https?://[^\s/$.?#].[^\s]*$"
patterns::VERSION_PATTERN    = r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$"
```

---

## Performance Thresholds

```rust
performance::MAX_WORKFLOW_STARTUP_TIME     = 100 ms
performance::MAX_NODE_EXECUTION_TIME       = 10 ms
performance::MAX_EXPRESSION_EVALUATION_TIME = 1 ms
performance::MAX_SERIALIZATION_TIME        = 5 ms
performance::MAX_DESERIALIZATION_TIME      = 5 ms
```

---

## Environment Variables

```rust
env::NEBULA_ENV          // "development" | "staging" | "production"
env::NEBULA_LOG_LEVEL    // "trace" | "debug" | "info" | "warn" | "error"
env::NEBULA_CONFIG_PATH  // Path to nebula.toml
env::NEBULA_DATABASE_URL
env::NEBULA_REDIS_URL
env::NEBULA_CLUSTER_NODES
env::NEBULA_TENANT_ID
env::NEBULA_USER_ID
```

---

## Default File Paths

```rust
paths::DEFAULT_CONFIG_DIR  = "config"
paths::DEFAULT_CONFIG_FILE = "nebula.toml"
paths::DEFAULT_LOG_DIR     = "logs"
paths::DEFAULT_DATA_DIR    = "data"
paths::DEFAULT_TEMP_DIR    = "temp"
paths::DEFAULT_CACHE_DIR   = "cache"
```

---

## Error Codes

```rust
error_codes::VALIDATION_ERROR         // "VALIDATION_ERROR"
error_codes::AUTHENTICATION_ERROR     // "AUTHENTICATION_ERROR"
error_codes::AUTHORIZATION_ERROR      // "AUTHORIZATION_ERROR"
error_codes::NOT_FOUND_ERROR          // "NOT_FOUND_ERROR"
error_codes::CONFLICT_ERROR           // "CONFLICT_ERROR"
error_codes::TIMEOUT_ERROR            // "TIMEOUT_ERROR"
error_codes::RATE_LIMIT_ERROR         // "RATE_LIMIT_ERROR"
error_codes::INTERNAL_ERROR           // "INTERNAL_ERROR"
error_codes::SERVICE_UNAVAILABLE_ERROR // "SERVICE_UNAVAILABLE_ERROR"
```

---

## Magic Bytes

```rust
magic::NEBULA_MAGIC        = b"NEBULA"
magic::NEBULA_MAGIC_LENGTH = 6
```

Used in binary file format headers to identify Nebula-produced files.
