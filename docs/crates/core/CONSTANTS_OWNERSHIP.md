# Constants Ownership (Phase 4)

Classification of constants for P-003: Reduce Core Constant Bloat.

## Foundation (keep in core)

| Constant(s) | Rationale |
|-------------|-----------|
| `SYSTEM_*` | Product identity; cross-cutting |
| `DEFAULT_TIMEOUT`, `DEFAULT_DATABASE_TIMEOUT`, `DEFAULT_HTTP_TIMEOUT`, `DEFAULT_GRPC_TIMEOUT` | Generic timeouts; used by many crates |
| `DEFAULT_MAX_RETRIES`, `DEFAULT_RETRY_DELAY`, `DEFAULT_MAX_RETRY_DELAY` | Generic retry; foundation for resilience |
| `env::*` | Environment variable names; cross-cutting |
| `paths::*` | Config paths; cross-cutting |
| `magic::*` | Binary format identifiers |
| `error_codes::*` | Error taxonomy; aligns with CoreError |
| `features::*` | Feature flags; cross-cutting |
| `patterns::*` | Validation patterns; foundation |
| `limits::*` (generic) | Cross-cutting limits (name length, etc.) |

## Domain-owned (move to owning crate)

| Constant(s) | Owning crate | Notes |
|-------------|--------------|-------|
| `DEFAULT_CIRCUIT_BREAKER_*`, `DEFAULT_BULKHEAD_*` | `nebula-resilience` | Circuit breaker, bulkhead |
| `DEFAULT_MAX_MEMORY_MB`, `DEFAULT_CACHE_*` | `nebula-memory` | Cache, memory |
| `DEFAULT_MAX_WORKFLOW_*`, `DEFAULT_MAX_EXECUTION_TIME` | `nebula-workflow` or `nebula-engine` | Workflow limits |
| `DEFAULT_MAX_NODE_*`, `DEFAULT_MAX_ACTION_*` | `nebula-action` | Node/action limits |
| `DEFAULT_MAX_EXPRESSION_*` | `nebula-expression` | Expression limits |
| `DEFAULT_MAX_EVENT_*` | `nebula-eventbus` | Event queue |
| `DEFAULT_MAX_STORAGE_*`, `DEFAULT_STORAGE_BATCH_SIZE` | `nebula-storage` | Storage limits |
| `DEFAULT_CLUSTER_*` | `nebula-cluster` (planned) | Cluster |
| `DEFAULT_MAX_TENANTS`, `DEFAULT_MAX_WORKFLOWS_PER_TENANT`, etc. | `nebula-tenant` (planned) | Tenant |
| `DEFAULT_API_*` | `nebula-api` | API limits |
| `DEFAULT_LOG_*` | `nebula-log` | Logging |
| `DEFAULT_METRICS_*` | `nebula-metrics` | Metrics |
| `DEFAULT_MAX_PASSWORD_*`, `DEFAULT_SESSION_*`, `DEFAULT_MAX_LOGIN_*` | `nebula-credential` or auth crate | Security |
| `DEFAULT_MAX_STRING_LENGTH`, `DEFAULT_MAX_ARRAY_SIZE`, `DEFAULT_MAX_OBJECT_PROPERTIES` | `nebula-validator` | Validation |
| `DEFAULT_MAX_SERIALIZATION_*` | Could stay in core or move to serialization crate | |
| `DEFAULT_TEST_*` | `nebula-sdk` or test utilities | Testing |
| `http::*` | `nebula-api` | HTTP status codes |
| `performance::*` | Domain crates (engine, expression, etc.) | Per-domain thresholds |
| `security::*` | Auth/credential crate | Security constants |

## Migration steps (P-003)

1. Create `constants` or `defaults` module in each owning crate.
2. Add constants with same values; document source.
3. In core: deprecate domain constants with `#[deprecated(note = "use nebula_xxx::constants::Y instead")]` and re-export for one minor cycle.
4. Update workspace usages to import from owning crates.
5. Remove deprecated re-exports in next major release.
