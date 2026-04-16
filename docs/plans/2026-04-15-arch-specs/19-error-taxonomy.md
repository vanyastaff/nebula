# Spec 19 — Error taxonomy at boundaries

> **Status:** draft
> **Canon target:** §12.4 (extend — error propagation contract), cross-ref to §3.10 nebula-error
> **Depends on:** existing `nebula-error` crate (fully implemented), 05 (API error response), 09 (retry classification), 18 (error correlation with trace_id)
> **Depended on by:** 20 (testing error patterns), any future SDK work

## Foundation is already there

`nebula-error` is **already implemented** with all core primitives. This spec does **not** rebuild it. It defines how the rest of the workspace uses it — what each layer's error type looks like, how conversions happen at boundaries, how the API maps to RFC 9457.

**What `nebula-error` gives us (v1 status: implemented):**

```rust
// Core traits and types
pub trait Classify {
    fn category(&self) -> ErrorCategory;
    fn code(&self) -> ErrorCode;
    fn severity(&self) -> ErrorSeverity { ErrorSeverity::Error }
    fn is_retryable(&self) -> bool { self.category().is_default_retryable() }
    fn retry_hint(&self) -> Option<RetryHint> { None }
}

pub enum ErrorCategory {
    NotFound, Validation, Authentication, Authorization, Conflict,
    RateLimit, Timeout, Exhausted, Cancelled, Internal, External,
    Unsupported, Unavailable, DataTooLarge,
}

pub struct ErrorCode(Cow<'static, str>);

pub mod codes {
    pub const NOT_FOUND: ErrorCode = ErrorCode::new("NOT_FOUND");
    pub const VALIDATION: ErrorCode = ErrorCode::new("VALIDATION");
    // ... 14 canonical codes matching categories
}

pub struct NebulaError<E: Classify> {
    inner: E,
    message: Option<Cow<'static, str>>,
    details: ErrorDetails,              // TypeId-keyed
    context_chain: Vec<Cow<'static, str>>,
    source: Option<Box<dyn Error + Send + Sync>>,
}

// Detail types (Google rpc-inspired)
pub struct ResourceInfo { resource_type, resource_name, owner }
pub struct BadRequest { violations: Vec<FieldViolation> }
pub struct DebugInfo { detail, stack_entries }
pub struct QuotaInfo { metric, limit, current, ... }
pub struct RetryHint { after, max_attempts, ... }
pub struct ExecutionContext { ... }
pub struct HelpLink { ... }
pub struct DependencyInfo { ... }
pub struct PreconditionFailure { violations }
pub struct RequestInfo { ... }
pub struct TypeMismatch { ... }
```

**What is missing (spec 19 adds):**

1. Per-layer domain enum types (`ActionError`, `RuntimeError`, `EngineError`, `ApiError`, `StorageError`, etc.)
2. Explicit `From` chain at each boundary
3. `nebula-api`'s `ApiError` → RFC 9457 renderer
4. PII discipline — which `ErrorDetails` are public-safe, which are internal
5. Recommended error code catalog beyond the 14 categorical ones
6. Integration with spec 18 observability context
7. Panic-handling contract

## Design principles

### Reuse `nebula-error::codes` as the canonical registry

The existing `codes::*` module holds 14 category-matched codes. Specific codes (like `WORKFLOW_NOT_FOUND`) can live in one of two places:

- **Centralised**: extend `nebula-error::codes` with additional specific codes
- **Distributed**: per-crate `pub const` in the crate that emits them

**Recommendation: distributed** for implementation codes, **centralised** for common codes shared across crates.

```rust
// In nebula-error::codes — shared, stable contract
pub mod codes {
    pub const NOT_FOUND: ErrorCode = ErrorCode::new("NOT_FOUND");
    // ... (already present)
    
    // Additions for workspace-wide use:
    pub const INSUFFICIENT_ROLE: ErrorCode = ErrorCode::new("INSUFFICIENT_ROLE");
    pub const QUOTA_EXCEEDED: ErrorCode = ErrorCode::new("QUOTA_EXCEEDED");
    pub const VERSION_MISMATCH: ErrorCode = ErrorCode::new("VERSION_MISMATCH");
    pub const LEASE_LOST: ErrorCode = ErrorCode::new("LEASE_LOST");
}

// In nebula-workflow — crate-specific codes
pub mod codes {
    use nebula_error::ErrorCode;
    pub const WORKFLOW_NOT_FOUND: ErrorCode = ErrorCode::new("WORKFLOW_NOT_FOUND");
    pub const WORKFLOW_VALIDATION_FAILED: ErrorCode = ErrorCode::new("WORKFLOW_VALIDATION_FAILED");
    pub const WORKFLOW_CYCLE_DETECTED: ErrorCode = ErrorCode::new("WORKFLOW_CYCLE_DETECTED");
    // ...
}
```

**Rule:** each code string is unique globally. A constant may be referenced from multiple crates but **defined** in only one. PR review enforces uniqueness.

### Per-crate error enum implementing `Classify`

Every crate that produces errors defines a **`thiserror`-derived enum** implementing `Classify`:

```rust
// nebula-workflow/src/error.rs
use nebula_error::{Classify, ErrorCategory, ErrorCode, codes};

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("workflow not found: {id}")]
    NotFound { id: WorkflowId },
    
    #[error("workflow version not found: {id}")]
    VersionNotFound { id: WorkflowVersionId },
    
    #[error("workflow validation failed: {violations} violations")]
    ValidationFailed { violations: usize },
    
    #[error("workflow cycle detected at node {node_id}")]
    CycleDetected { node_id: String },
    
    #[error("workflow has no published version")]
    NoPublishedVersion,
    
    #[error("schema version {schema_version} not supported")]
    UnsupportedSchema { schema_version: u16 },
}

impl Classify for WorkflowError {
    fn category(&self) -> ErrorCategory {
        use ErrorCategory::*;
        match self {
            Self::NotFound { .. } | Self::VersionNotFound { .. } | Self::NoPublishedVersion => NotFound,
            Self::ValidationFailed { .. } | Self::CycleDetected { .. } => Validation,
            Self::UnsupportedSchema { .. } => Unsupported,
        }
    }
    
    fn code(&self) -> ErrorCode {
        use crate::codes::*;
        match self {
            Self::NotFound { .. } => WORKFLOW_NOT_FOUND,
            Self::VersionNotFound { .. } => WORKFLOW_VERSION_NOT_FOUND,
            Self::ValidationFailed { .. } => WORKFLOW_VALIDATION_FAILED,
            Self::CycleDetected { .. } => WORKFLOW_CYCLE_DETECTED,
            Self::NoPublishedVersion => WORKFLOW_NOT_PUBLISHED,
            Self::UnsupportedSchema { .. } => WORKFLOW_SCHEMA_UNSUPPORTED,
        }
    }
}
```

Or via `#[derive(Classify)]` if the macro supports per-variant attributes:

```rust
#[derive(Debug, thiserror::Error, Classify)]
pub enum WorkflowError {
    #[error("workflow not found: {id}")]
    #[classify(category = "NotFound", code = "WORKFLOW_NOT_FOUND")]
    NotFound { id: WorkflowId },
    
    #[error("workflow validation failed")]
    #[classify(category = "Validation", code = "WORKFLOW_VALIDATION_FAILED")]
    ValidationFailed { violations: Vec<String> },
    
    // ...
}
```

**Macro design is up to `nebula-error-macros` implementor.** Manual `impl Classify` is always available as fallback.

### Use `NebulaError<E>` at boundaries

Inside a crate, code returns `Result<T, MyDomainError>`. At a **layer boundary** (entering another crate), wrap in `NebulaError<E>` to attach context:

```rust
// Inside nebula-workflow
pub fn validate_workflow(def: &WorkflowDefinition) -> Result<(), WorkflowError> {
    // Internal: plain domain error
    if has_cycle(def) {
        return Err(WorkflowError::CycleDetected { node_id: /* ... */ });
    }
    Ok(())
}

// At the engine layer consuming this
impl EngineService {
    pub async fn publish_workflow(
        &self,
        ctx: &TenantContext,
        workflow_id: WorkflowId,
        draft_version_id: WorkflowVersionId,
    ) -> Result<(), NebulaError<EngineError>> {
        // Load + validate — wrap domain error at boundary
        let version = self.repo
            .load_version(&draft_version_id)
            .await
            .map_err(|e| NebulaError::new(EngineError::StorageFailed)
                .with_source(e)
                .context("loading draft version for publish")
            )?;
        
        validate_workflow(&version.definition)
            .map_err(|e| NebulaError::new(EngineError::PlanValidationFailed)
                .with_source(e)
                .context("validating draft before publish")
                .with_detail(BadRequest {
                    violations: vec![/* ... */],
                })
            )?;
        
        // ...
        Ok(())
    }
}
```

The **context_chain** accumulates as the error bubbles up. `NebulaError::Display` renders as `outer → inner → root`.

### When NOT to wrap

Plain domain enums within a crate don't need `NebulaError` — it's only at **cross-crate** boundaries. Internal use:

```rust
// Inside nebula-storage — plain storage error
async fn fetch_row(id: &[u8]) -> Result<Row, StorageError> {
    sqlx::query_as!(...).fetch_one(&self.pool).await.map_err(StorageError::from)
}
```

When the storage crate's function is called from engine, engine wraps the `StorageError` into `NebulaError<EngineError>` at boundary. **One wrapping per boundary**, not per function call.

## Six-layer propagation

### Layer 1: External libraries

Third-party crate errors (`reqwest::Error`, `sqlx::Error`, `serde_json::Error`, `tokio::io::Error`, etc.) are **immediately converted** to domain errors when crossing into Nebula code:

```rust
// In nebula-storage
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("pool timeout")]
    PoolTimeout,
    
    #[error("database unavailable")]
    Unavailable,
    
    #[error("unique violation on {constraint}")]
    UniqueViolation { constraint: String },
    
    #[error("version mismatch (CAS failure)")]
    VersionMismatch,
    
    #[error("serialization error: {0}")]
    Serialization(String),
    
    #[error("query failed: {0}")]
    QueryFailed(String),
}

impl From<sqlx::Error> for StorageError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::PoolTimedOut => Self::PoolTimeout,
            sqlx::Error::Io(_) | sqlx::Error::Tls(_) => Self::Unavailable,
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                Self::UniqueViolation {
                    constraint: db_err.constraint().unwrap_or("unknown").to_string(),
                }
            }
            sqlx::Error::RowNotFound => {
                // Note: NotFound doesn't have its own StorageError variant —
                // storage treats it as "absence of row", caller decides if it's an error
                Self::QueryFailed(format!("row not found"))
            }
            _ => Self::QueryFailed(e.to_string()),
        }
    }
}

impl Classify for StorageError {
    fn category(&self) -> ErrorCategory {
        match self {
            Self::PoolTimeout => ErrorCategory::Timeout,
            Self::Unavailable => ErrorCategory::Unavailable,
            Self::UniqueViolation { .. } => ErrorCategory::Conflict,
            Self::VersionMismatch => ErrorCategory::Conflict,
            Self::Serialization(_) | Self::QueryFailed(_) => ErrorCategory::Internal,
        }
    }
    
    fn code(&self) -> ErrorCode {
        match self {
            Self::PoolTimeout => codes::TIMEOUT,
            Self::Unavailable => codes::UNAVAILABLE,
            Self::UniqueViolation { .. } => codes::CONFLICT,
            Self::VersionMismatch => codes::VERSION_MISMATCH,
            Self::Serialization(_) | Self::QueryFailed(_) => codes::INTERNAL,
        }
    }
}
```

**Classification happens at conversion time**. The moment a third-party error enters the wrapper, `Classify` answers retryability for downstream consumers.

**Rule:** never bubble a raw third-party error across a module boundary. Wrap at the first `?` in the function that touches the library.

### Layer 2: Action code (user implementation)

From spec 09, `ActionError` is the **only** error type authors return from `execute()`:

```rust
// nebula-action/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("transient failure: {0}")]
    Transient(String),
    
    #[error("transient failure: {message} (retry after {retry_after:?})")]
    TransientWithHint { message: String, retry_after: Duration },
    
    #[error("permanent failure: {0}")]
    Permanent(String),
    
    #[error("cancelled")]
    Cancelled,
    
    #[error("cancelled (escalated)")]
    CancelledEscalated,
    
    #[error("fatal: {0}")]
    Fatal(String),
    
    #[error("timeout")]
    Timeout,
}

impl Classify for ActionError {
    fn category(&self) -> ErrorCategory {
        use ErrorCategory::*;
        match self {
            Self::Transient(_) | Self::TransientWithHint { .. } => External,
            Self::Permanent(_) => Validation,
            Self::Cancelled | Self::CancelledEscalated => Cancelled,
            Self::Fatal(_) => Internal,
            Self::Timeout => Timeout,
        }
    }
    
    fn code(&self) -> ErrorCode {
        match self {
            Self::Transient(_) => codes::ACTION_TRANSIENT,
            Self::TransientWithHint { .. } => codes::ACTION_TRANSIENT,
            Self::Permanent(_) => codes::ACTION_PERMANENT,
            Self::Cancelled => codes::CANCELLED,
            Self::CancelledEscalated => codes::ACTION_CANCELLED_ESCALATED,
            Self::Fatal(_) => codes::ACTION_FATAL,
            Self::Timeout => codes::TIMEOUT,
        }
    }
    
    fn is_retryable(&self) -> bool {
        match self {
            Self::Transient(_) | Self::TransientWithHint { .. } | Self::Timeout => true,
            _ => false,
        }
    }
    
    fn retry_hint(&self) -> Option<RetryHint> {
        match self {
            Self::TransientWithHint { retry_after, .. } => {
                Some(RetryHint::after(*retry_after))
            }
            _ => None,
        }
    }
}
```

**Authors never return any other type.** This is enforced by the `Action` trait's `execute` signature.

### Layer 3: Runtime

Runtime handles action results. Wraps into `RuntimeError` with context:

```rust
// nebula-runtime/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("action {action_key} returned error at attempt {attempt}")]
    ActionReturnedError {
        action_key: ActionKey,
        attempt_id: AttemptId,
        attempt: u32,
    },
    
    #[error("action {action_key} panicked at attempt {attempt}")]
    ActionPanicked {
        action_key: ActionKey,
        attempt_id: AttemptId,
        attempt: u32,
        panic_message: String,   // sanitized (no stack trace, no PII)
    },
    
    #[error("state persistence failed for attempt {attempt_id}")]
    StatePersistenceFailed { attempt_id: AttemptId },
    
    #[error("checkpoint failed for attempt {attempt_id}")]
    CheckpointFailed { attempt_id: AttemptId },
    
    #[error("lease lost for attempt {attempt_id}")]
    LeaseLost { attempt_id: AttemptId },
    
    #[error("state schema incompatible: expected {expected}, got {actual}")]
    StateSchemaIncompatible {
        attempt_id: AttemptId,
        expected: String,
        actual: String,
    },
}

impl Classify for RuntimeError {
    fn category(&self) -> ErrorCategory {
        use ErrorCategory::*;
        match self {
            Self::ActionReturnedError { .. } => External,
            Self::ActionPanicked { .. } => Internal,
            Self::StatePersistenceFailed { .. } | Self::CheckpointFailed { .. } => Internal,
            Self::LeaseLost { .. } => Conflict,
            Self::StateSchemaIncompatible { .. } => Unsupported,
        }
    }
    
    fn code(&self) -> ErrorCode {
        match self {
            Self::ActionReturnedError { .. } => codes::ACTION_RETURNED_ERROR,
            Self::ActionPanicked { .. } => codes::ACTION_PANICKED,
            Self::StatePersistenceFailed { .. } => codes::STATE_PERSISTENCE_FAILED,
            Self::CheckpointFailed { .. } => codes::CHECKPOINT_FAILED,
            Self::LeaseLost { .. } => codes::LEASE_LOST,
            Self::StateSchemaIncompatible { .. } => codes::STATE_SCHEMA_INCOMPATIBLE,
        }
    }
}
```

Conversions from lower layers:

```rust
impl RuntimeError {
    pub fn wrap_action_error(
        err: ActionError,
        action_key: ActionKey,
        attempt_id: AttemptId,
        attempt: u32,
    ) -> NebulaError<RuntimeError> {
        let runtime_err = RuntimeError::ActionReturnedError {
            action_key,
            attempt_id,
            attempt,
        };
        
        // Preserve the ActionError as source
        NebulaError::new(runtime_err)
            .with_source(err)  // <- Original ActionError preserved in source chain
            .context("action returned error during attempt")
    }
}
```

The **original `ActionError`** is preserved in `source` chain. Retry logic (spec 09) walks source chain to pull `ActionError` and decide retryability.

### Layer 4: Engine / services

```rust
// nebula-engine/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("workflow not found: {0}")]
    WorkflowNotFound(WorkflowId),
    
    #[error("execution not found: {0}")]
    ExecutionNotFound(ExecutionId),
    
    #[error("execution {execution_id} not claimable from state {status:?}")]
    ExecutionNotClaimable {
        execution_id: ExecutionId,
        status: ExecutionStatus,
    },
    
    #[error("plan validation failed ({0} errors)")]
    PlanValidationFailed(usize),
    
    #[error("quota exceeded: {kind}")]
    QuotaExceeded { kind: QuotaKind, limit: u64, current: u64 },
    
    #[error("runtime failed for attempt {attempt_id}")]
    RuntimeFailed { attempt_id: AttemptId },
    
    #[error("storage failed")]
    StorageFailed,
    
    #[error("execution orphaned after {takeover_count} takeovers")]
    ExecutionOrphaned {
        execution_id: ExecutionId,
        takeover_count: u32,
    },
}

impl Classify for EngineError {
    fn category(&self) -> ErrorCategory {
        use ErrorCategory::*;
        match self {
            Self::WorkflowNotFound(_) | Self::ExecutionNotFound(_) => NotFound,
            Self::ExecutionNotClaimable { .. } => Conflict,
            Self::PlanValidationFailed(_) => Validation,
            Self::QuotaExceeded { .. } => Exhausted,
            Self::RuntimeFailed { .. } | Self::StorageFailed | Self::ExecutionOrphaned { .. } => Internal,
        }
    }
    
    fn code(&self) -> ErrorCode {
        match self {
            Self::WorkflowNotFound(_) => codes::WORKFLOW_NOT_FOUND,
            Self::ExecutionNotFound(_) => codes::EXECUTION_NOT_FOUND,
            Self::ExecutionNotClaimable { .. } => codes::EXECUTION_NOT_CANCELLABLE,
            Self::PlanValidationFailed(_) => codes::WORKFLOW_VALIDATION_FAILED,
            Self::QuotaExceeded { .. } => codes::QUOTA_EXCEEDED,
            Self::RuntimeFailed { .. } => codes::ACTION_RETURNED_ERROR,
            Self::StorageFailed => codes::STORAGE_UNAVAILABLE,
            Self::ExecutionOrphaned { .. } => codes::EXECUTION_ORPHANED,
        }
    }
}
```

Engine conversions:

```rust
// Engine operation wraps RuntimeError
impl EngineService {
    pub async fn run_node(&self, ...) -> Result<Value, NebulaError<EngineError>> {
        self.runtime
            .run_action(ctx, input)
            .await
            .map_err(|runtime_err: NebulaError<RuntimeError>| {
                NebulaError::new(EngineError::RuntimeFailed { attempt_id })
                    .with_source(runtime_err)
                    .context("engine running action")
            })
    }
}
```

Note: `NebulaError<RuntimeError>` as source of `NebulaError<EngineError>`. Double-wrapping preserves both `context_chain` layers. Display shows `engine running action → action returned error → underlying`.

### Layer 5: API (HTTP boundary)

```rust
// nebula-api/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {resource}")]
    NotFound {
        resource: &'static str,
        identifier: String,
    },
    
    #[error("insufficient role: required {required}, current {current:?}")]
    InsufficientRole {
        required: &'static str,
        current: Option<&'static str>,
    },
    
    #[error("not authenticated")]
    Unauthenticated,
    
    #[error("session expired")]
    SessionExpired,
    
    #[error("rate limited, retry in {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    
    #[error("quota exceeded: {kind}")]
    QuotaExceeded { kind: QuotaKind },
    
    #[error("validation failed")]
    ValidationFailed { errors: Vec<FieldViolation> },
    
    #[error("conflict on {resource}: {reason}")]
    Conflict { resource: &'static str, reason: &'static str },
    
    #[error("upstream error")]
    Upstream { service: &'static str, safe_message: String },
    
    #[error("internal error (correlation: {trace_id})")]
    Internal {
        error_code: ErrorCode,
        trace_id: String,
        // Original error stored but NOT in the Display impl (PII safety)
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl ApiError {
    pub fn http_status(&self) -> http::StatusCode {
        use http::StatusCode as S;
        match self {
            Self::NotFound { .. } => S::NOT_FOUND,
            Self::InsufficientRole { .. } => S::FORBIDDEN,
            Self::Unauthenticated | Self::SessionExpired => S::UNAUTHORIZED,
            Self::RateLimited { .. } => S::TOO_MANY_REQUESTS,
            Self::QuotaExceeded { .. } => S::PAYMENT_REQUIRED,
            Self::ValidationFailed { .. } => S::UNPROCESSABLE_ENTITY,
            Self::Conflict { .. } => S::CONFLICT,
            Self::Upstream { .. } => S::BAD_GATEWAY,
            Self::Internal { .. } => S::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Classify for ApiError {
    fn category(&self) -> ErrorCategory {
        use ErrorCategory::*;
        match self {
            Self::NotFound { .. } => NotFound,
            Self::InsufficientRole { .. } => Authorization,
            Self::Unauthenticated | Self::SessionExpired => Authentication,
            Self::RateLimited { .. } => RateLimit,
            Self::QuotaExceeded { .. } => Exhausted,
            Self::ValidationFailed { .. } => Validation,
            Self::Conflict { .. } => Conflict,
            Self::Upstream { .. } => External,
            Self::Internal { .. } => Internal,
        }
    }
    
    fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound { resource: "workflow", .. } => codes::WORKFLOW_NOT_FOUND,
            Self::NotFound { resource: "execution", .. } => codes::EXECUTION_NOT_FOUND,
            Self::NotFound { .. } => codes::NOT_FOUND,
            Self::InsufficientRole { .. } => codes::INSUFFICIENT_ROLE,
            Self::Unauthenticated => codes::NOT_AUTHENTICATED,
            Self::SessionExpired => codes::SESSION_EXPIRED,
            Self::RateLimited { .. } => codes::RATE_LIMIT,
            Self::QuotaExceeded { .. } => codes::QUOTA_EXCEEDED,
            Self::ValidationFailed { .. } => codes::VALIDATION,
            Self::Conflict { .. } => codes::CONFLICT,
            Self::Upstream { .. } => codes::EXTERNAL,
            Self::Internal { error_code, .. } => error_code.clone(),
        }
    }
}

impl From<NebulaError<EngineError>> for ApiError {
    fn from(err: NebulaError<EngineError>) -> Self {
        // 1. ALWAYS log internally first with full fidelity
        tracing::error!(
            error.code = %err.code(),
            error.category = ?err.category(),
            error.retryable = err.is_retryable(),
            error.message = %err,
            error.debug = ?err,
            source = ?err.source(),
            "engine operation failed"
        );
        
        // 2. Map to public variant
        match err.inner() {
            EngineError::WorkflowNotFound(id) => ApiError::NotFound {
                resource: "workflow",
                identifier: id.to_string(),
            },
            EngineError::ExecutionNotFound(id) => ApiError::NotFound {
                resource: "execution",
                identifier: id.to_string(),
            },
            EngineError::QuotaExceeded { kind, .. } => ApiError::QuotaExceeded {
                kind: kind.clone(),
            },
            EngineError::PlanValidationFailed(_) => {
                // Pull BadRequest detail if available
                let errors = err.detail::<BadRequest>()
                    .map(|br| br.violations.clone())
                    .unwrap_or_default();
                ApiError::ValidationFailed { errors }
            }
            // Catch-all: opaque internal error
            _ => ApiError::Internal {
                error_code: err.code(),
                trace_id: current_trace_id().to_string(),
                source: None,  // not leaked through Display
            },
        }
    }
}
```

**Two-tier projection is enforced by `ApiError::From` impl**: internal logging happens before conversion; public variant carries only safe fields.

### Layer 6: Customer (RFC 9457 response)

`ApiError` → `ProblemJson` response:

```rust
// nebula-api/src/problem_json.rs
#[derive(Debug, Serialize)]
pub struct ProblemJson {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    pub instance: Option<String>,
    pub error_code: String,
    pub trace_id: Option<String>,
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<serde_json::Value>,
}

impl ApiError {
    pub fn into_problem_json(self, obs_ctx: &ObservabilityContext) -> ProblemJson {
        let status = self.http_status().as_u16();
        let code = self.code();
        
        ProblemJson {
            type_uri: format!("https://nebula.io/errors/{}", code.as_str().to_lowercase().replace('_', "-")),
            title: self.public_title().to_string(),
            status,
            detail: self.public_detail(),
            instance: obs_ctx.request_id.as_ref().map(|id| format!("req/{}", id)),
            error_code: code.to_string(),
            trace_id: obs_ctx.trace_id.map(|t| format!("{:032x}", t)),
            request_id: obs_ctx.request_id.clone(),
            details: self.public_details(),
        }
    }
    
    fn public_title(&self) -> &'static str {
        match self {
            Self::NotFound { resource, .. } => "Resource not found",
            Self::InsufficientRole { .. } => "Insufficient role",
            Self::Unauthenticated => "Not authenticated",
            Self::SessionExpired => "Session expired",
            Self::RateLimited { .. } => "Rate limit exceeded",
            Self::QuotaExceeded { .. } => "Quota exceeded",
            Self::ValidationFailed { .. } => "Validation failed",
            Self::Conflict { .. } => "Conflict",
            Self::Upstream { .. } => "Upstream service error",
            Self::Internal { .. } => "Internal server error",
        }
    }
    
    fn public_detail(&self) -> String {
        match self {
            Self::NotFound { resource, identifier } => {
                format!("{} `{}` not found", resource, identifier)
            }
            Self::InsufficientRole { required, current } => {
                format!("requires role {}, have {:?}", required, current)
            }
            Self::RateLimited { retry_after_secs } => {
                format!("try again in {} seconds", retry_after_secs)
            }
            Self::QuotaExceeded { kind } => {
                format!("{} quota exceeded", kind)
            }
            Self::ValidationFailed { errors } => {
                format!("{} validation errors", errors.len())
            }
            Self::Internal { trace_id, .. } => {
                format!("internal error — reference trace ID {}", trace_id)
            }
            _ => self.to_string(),
        }
    }
    
    fn public_details(&self) -> Vec<serde_json::Value> {
        match self {
            Self::ValidationFailed { errors } => {
                vec![serde_json::json!({
                    "kind": "field_violations",
                    "violations": errors.iter().map(|v| json!({
                        "field": v.field,
                        "description": v.description,
                        "code": v.code.to_string(),
                    })).collect::<Vec<_>>(),
                })]
            }
            Self::RateLimited { retry_after_secs } => {
                vec![serde_json::json!({
                    "kind": "retry_hint",
                    "retry_after_ms": retry_after_secs * 1000,
                })]
            }
            Self::QuotaExceeded { kind } => {
                vec![serde_json::json!({
                    "kind": "quota",
                    "quota_kind": kind.to_string(),
                })]
            }
            _ => vec![],
        }
    }
}
```

## PII rules — which detail types are public-safe

`nebula-error`'s existing detail types have different safety profiles:

| Detail type | Safe for public API? | Why |
|---|---|---|
| `ResourceInfo` | **safe** | resource_type + resource_name; no owner if optional |
| `BadRequest` + `FieldViolation` | **safe** | field names and validation messages; scrub values from violation descriptions |
| `QuotaInfo` | **safe** | metric name, limits, current usage |
| `RetryHint` | **safe** | backoff timing |
| `HelpLink` | **safe** | URL to docs |
| `DependencyInfo` | **partially** | depends on whether dependency names are public |
| `PreconditionFailure` | **safe** | structured precondition that failed |
| `RequestInfo` | **safe** | request id, not request body |
| `TypeMismatch` | **safe** | expected vs actual type names |
| `ErrorRoute` | **partially** | depends on whether route contains internal path |
| `DebugInfo` | **NEVER** | detail string + stack entries are internal only |
| `ExecutionContext` | **partially** | execution_id ok, internal state not |

**Rule enforced at API boundary:** `into_problem_json()` only serializes `details` from the **safe list**. Any other detail types are dropped with a log warning in internal logging.

```rust
fn safe_details(&self) -> Vec<serde_json::Value> {
    let mut out = vec![];
    
    if let Some(ri) = self.detail::<ResourceInfo>() {
        out.push(serde_json::to_value(ri).unwrap());
    }
    if let Some(br) = self.detail::<BadRequest>() {
        out.push(serde_json::to_value(br).unwrap());
    }
    // ... explicit allowlist
    
    // DebugInfo explicitly skipped — it's for internal logging only
    
    out
}
```

**`DebugInfo` never reaches the wire.** Author can attach it for internal tracing, but public projection drops it.

## Panic handling

Rule: **panics never leave action boundary** except as `Fatal`. Implementation:

```rust
// nebula-runtime/src/executor.rs
pub async fn run_action(
    action: Arc<dyn Action>,
    ctx: ActionContext,
    input: Value,
    cancel_grace: Duration,
) -> Result<Value, NebulaError<RuntimeError>> {
    let action_key = action.key();
    let attempt_id = ctx.attempt_id;
    let attempt = ctx.attempt_number;
    
    let handle = tokio::spawn({
        let ctx = ctx.clone();
        async move { action.execute(ctx, input).await }
    });
    
    match handle.await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(action_err)) => {
            Err(NebulaError::new(RuntimeError::ActionReturnedError {
                action_key, attempt_id, attempt,
            })
            .with_source(action_err))
        }
        Err(join_err) if join_err.is_panic() => {
            // Sanitize panic message — no PII, no stack
            let panic_msg = extract_panic_message_sanitized(&join_err);
            
            // Log internally with full stack
            tracing::error!(
                error.code = "ACTION_PANICKED",
                action_key = %action_key,
                attempt_id = %attempt_id,
                panic_message = %panic_msg,
                join_error = ?join_err,
                "action panicked"
            );
            
            // Metric
            metrics::counter!("nebula_action_panics_total", "action_kind" => action_key.kind()).increment(1);
            
            Err(NebulaError::new(RuntimeError::ActionPanicked {
                action_key, attempt_id, attempt,
                panic_message: panic_msg,
            }).context("action panicked during execution"))
        }
        Err(join_err) if join_err.is_cancelled() => {
            // Task dropped by runtime (escalation path)
            Err(NebulaError::new(RuntimeError::ActionReturnedError {
                action_key, attempt_id, attempt,
            })
            .with_source(ActionError::CancelledEscalated))
        }
        Err(join_err) => {
            Err(NebulaError::new(RuntimeError::ActionReturnedError {
                action_key, attempt_id, attempt,
            })
            .with_source(ActionError::Fatal(format!("task error: {}", join_err))))
        }
    }
}

fn extract_panic_message_sanitized(err: &tokio::task::JoinError) -> String {
    // Get panic payload if available
    // Note: we intentionally don't include file:line, stack, or any env info
    // Operator checks internal logs for full detail; public gets opaque message
    "action panicked (see internal logs for details)".to_string()
}
```

**Sanitization rules:**

- Panic message from user code **may** contain PII (user input, credential values)
- Never include raw panic message in user-facing responses
- Internal logs can include full `?join_err` for debugging
- Journal / audit entries include sanitized «action panicked» marker only

## Integration with spec 18 (observability)

Every error emission uses structured fields via `tracing`:

```rust
// Standardized error logging helper
#[macro_export]
macro_rules! error_with_context {
    ($err:expr, $msg:literal) => {{
        let err = &$err;
        tracing::error!(
            error.code = %err.code(),
            error.category = ?err.category(),
            error.retryable = err.is_retryable(),
            error.severity = ?err.severity(),
            error = %err,
            error.debug = ?err,
            $msg
        );
    }};
}
```

Every error entering a log line automatically gets `error.code`, `error.category`, `error.retryable` as structured fields. Combined with `ObservabilityContext` attributes (trace_id, execution_id, org_id), operators can grep by error_code in Loki or run aggregation queries:

```logql
{service="nebula"} | error.code = "QUOTA_EXCEEDED" | execution_id="exec_01J9..."
```

Errors also show up in metrics via allowlisted labels:

```rust
metrics::counter!(
    "nebula_errors_total",
    "category" => format!("{:?}", err.category()),
    "code" => err.code().to_string(),
    "retryable" => err.is_retryable().to_string(),
).increment(1);
```

Cardinality acceptable — categories are ~14, codes are ~50 in v1.

## Recommended error code catalog (v1)

Beyond the 14 categorical codes already in `nebula-error::codes`, add these to the centralised catalog or as per-crate constants:

### Tenancy / identity (`nebula-core` or `nebula-error::codes`)

```rust
pub const INSUFFICIENT_ROLE: ErrorCode = ErrorCode::new("INSUFFICIENT_ROLE");
pub const NOT_AUTHENTICATED: ErrorCode = ErrorCode::new("NOT_AUTHENTICATED");
pub const SESSION_EXPIRED: ErrorCode = ErrorCode::new("SESSION_EXPIRED");
pub const MFA_REQUIRED: ErrorCode = ErrorCode::new("MFA_REQUIRED");
pub const INVALID_CREDENTIALS: ErrorCode = ErrorCode::new("INVALID_CREDENTIALS");
pub const ACCOUNT_LOCKED: ErrorCode = ErrorCode::new("ACCOUNT_LOCKED");
```

### Workflow (`nebula-workflow::codes`)

```rust
pub const WORKFLOW_NOT_FOUND: ErrorCode = ErrorCode::new("WORKFLOW_NOT_FOUND");
pub const WORKFLOW_VERSION_NOT_FOUND: ErrorCode = ErrorCode::new("WORKFLOW_VERSION_NOT_FOUND");
pub const WORKFLOW_NOT_PUBLISHED: ErrorCode = ErrorCode::new("WORKFLOW_NOT_PUBLISHED");
pub const WORKFLOW_VALIDATION_FAILED: ErrorCode = ErrorCode::new("WORKFLOW_VALIDATION_FAILED");
pub const WORKFLOW_CYCLE_DETECTED: ErrorCode = ErrorCode::new("WORKFLOW_CYCLE_DETECTED");
pub const WORKFLOW_SCHEMA_UNSUPPORTED: ErrorCode = ErrorCode::new("WORKFLOW_SCHEMA_UNSUPPORTED");
pub const EXPRESSION_COMPILE_ERROR: ErrorCode = ErrorCode::new("EXPRESSION_COMPILE_ERROR");
```

### Execution / runtime (`nebula-execution::codes` or `nebula-runtime::codes`)

```rust
pub const EXECUTION_NOT_FOUND: ErrorCode = ErrorCode::new("EXECUTION_NOT_FOUND");
pub const EXECUTION_NOT_CANCELLABLE: ErrorCode = ErrorCode::new("EXECUTION_NOT_CANCELLABLE");
pub const EXECUTION_ORPHANED: ErrorCode = ErrorCode::new("EXECUTION_ORPHANED");
pub const ACTION_RETURNED_ERROR: ErrorCode = ErrorCode::new("ACTION_RETURNED_ERROR");
pub const ACTION_TRANSIENT: ErrorCode = ErrorCode::new("ACTION_TRANSIENT");
pub const ACTION_PERMANENT: ErrorCode = ErrorCode::new("ACTION_PERMANENT");
pub const ACTION_FATAL: ErrorCode = ErrorCode::new("ACTION_FATAL");
pub const ACTION_PANICKED: ErrorCode = ErrorCode::new("ACTION_PANICKED");
pub const ACTION_CANCELLED_ESCALATED: ErrorCode = ErrorCode::new("ACTION_CANCELLED_ESCALATED");
pub const RETRY_BUDGET_EXHAUSTED: ErrorCode = ErrorCode::new("RETRY_BUDGET_EXHAUSTED");
pub const STATEFUL_MAX_DURATION_EXCEEDED: ErrorCode = ErrorCode::new("STATEFUL_MAX_DURATION_EXCEEDED");
pub const STATE_SCHEMA_INCOMPATIBLE: ErrorCode = ErrorCode::new("STATE_SCHEMA_INCOMPATIBLE");
pub const STATE_PERSISTENCE_FAILED: ErrorCode = ErrorCode::new("STATE_PERSISTENCE_FAILED");
pub const CHECKPOINT_FAILED: ErrorCode = ErrorCode::new("CHECKPOINT_FAILED");
pub const LEASE_LOST: ErrorCode = ErrorCode::new("LEASE_LOST");
```

### Quotas / rate limiting (`nebula-error::codes`)

```rust
pub const QUOTA_EXCEEDED: ErrorCode = ErrorCode::new("QUOTA_EXCEEDED");
pub const MONTHLY_QUOTA_EXCEEDED: ErrorCode = ErrorCode::new("MONTHLY_QUOTA_EXCEEDED");
pub const STORAGE_QUOTA_EXCEEDED: ErrorCode = ErrorCode::new("STORAGE_QUOTA_EXCEEDED");
```

### Storage (`nebula-storage::codes`)

```rust
pub const STORAGE_UNAVAILABLE: ErrorCode = ErrorCode::new("STORAGE_UNAVAILABLE");
pub const VERSION_MISMATCH: ErrorCode = ErrorCode::new("VERSION_MISMATCH");
pub const DUPLICATE_SLUG: ErrorCode = ErrorCode::new("DUPLICATE_SLUG");
```

### Triggers (`nebula-action::codes` or `nebula-trigger::codes`)

```rust
pub const TRIGGER_EVENT_DEDUPLICATED: ErrorCode = ErrorCode::new("TRIGGER_EVENT_DEDUPLICATED");
pub const WEBHOOK_AUTH_FAILED: ErrorCode = ErrorCode::new("WEBHOOK_AUTH_FAILED");
pub const WEBHOOK_REPLAY_REJECTED: ErrorCode = ErrorCode::new("WEBHOOK_REPLAY_REJECTED");
```

**Total v1 catalog:** ~50 codes. PR review enforces uniqueness and additions. Document URL per code links to dedicated docs page.

## Configuration surface

```toml
[errors]
# Include stack traces in internal logs for non-panic errors (expensive)
include_source_chain_in_logs = true

# Maximum error detail size in API responses (anti-leak)
max_public_detail_length = 1000

# Strip source chain from public responses (always true in production)
strip_source_in_public = true

# Docs URL base for error types (RFC 9457 `type` field)
docs_url_base = "https://nebula.io/errors"
```

## Testing criteria

**Unit tests:**

- Each layer's error enum implements `Classify` for every variant
- `From` impls preserve context via `source` chain
- `ApiError::into_problem_json` never leaks `DebugInfo` in details
- `ApiError::http_status` matches documented status codes
- `ApiError::code` returns stable strings (never changes)
- Panic sanitization strips file paths, line numbers, env values

**Integration tests:**

- Simulate action returning `ActionError::Transient` → trace error through runtime → engine → API → verify 502 + `ACTION_TRANSIENT` code
- Simulate action panic → verify `ACTION_PANICKED` code + sanitized message + internal log has full stack
- Simulate quota exceeded → verify 402 + `QUOTA_EXCEEDED` + `QuotaInfo` detail in response
- Simulate validation failure → verify 422 + `BadRequest` detail with field violations
- Verify `trace_id` present in every error response

**Contract tests (vs. docs):**

- Every code constant has an entry in `docs/errors/` with RFC 9457 type URI
- Every error enum variant has at least one test covering its Classify output

**PII leak tests:**

- Inject credential value into error chain → verify not present in public response
- Inject user PII (email) into panic → verify stripped from `panic_message`
- Internal log contains full chain, public response does not

## Performance targets

- `Classify::code()` lookup: **< 100 ns** (match expression, no allocations)
- `NebulaError::new()`: **< 500 ns** (no details, no context)
- `NebulaError::with_detail()`: **< 1 µs** per detail
- `into_problem_json()`: **< 50 µs** for typical error
- Error logging overhead: **< 5 µs** per error (tracing macro)

## Module boundaries

| Component | Crate |
|---|---|
| `Classify`, `ErrorCategory`, `ErrorCode`, `codes`, `ErrorSeverity`, `NebulaError`, `ErrorDetails`, detail types | `nebula-error` (existing) |
| `#[derive(Classify)]` macro | `nebula-error-macros` (existing) |
| `ActionError` + `Classify` impl | `nebula-action` |
| `RuntimeError` + `Classify` impl | `nebula-runtime` |
| `EngineError` + `Classify` impl | `nebula-engine` |
| `WorkflowError` + `Classify` impl | `nebula-workflow` |
| `StorageError` + `Classify` impl | `nebula-storage` |
| `ApiError` + `Classify` impl + `ProblemJson` renderer | `nebula-api` |
| `AuthError` (v1.5+) | `nebula-auth` |
| `SandboxError` | `nebula-sandbox` |
| Plugin errors | per-plugin, using `Classify` from `nebula-error` |

## Migration path

- **`nebula-error` is already built** — no changes needed to the foundation
- **Per-crate errors**: some crates have partial error types (`EngineError`, `WorkflowError`, etc.). Audit existing types, refine to match spec's required variants, ensure all implement `Classify`
- **Add missing codes** to catalog through PRs (or as per-crate constants)
- **API layer**: `ApiError` enum and `ProblemJson` renderer are new — build in `nebula-api`
- **Panic handling**: wrap `tokio::spawn` with sanitization helper in `nebula-runtime`
- **CI rules**:
  - Deny `anyhow` in non-binary crates (via `cargo deny`)
  - Deny `Box<dyn Error>` in public APIs (via lint or review)
  - Require `Classify` impl on all public error enums (review)
  - PII test fixtures verify no credentials leak into error responses

## Canon §12.4 extension

Proposed addition to canon:

```markdown
### 12.4 Errors and contracts (extended)

Library crates use `thiserror` typed enums implementing `nebula_error::Classify`.
Cross-crate boundaries wrap errors in `NebulaError<E>` with explicit `context_chain`
entries. API boundary maps to `ApiError` which serializes as RFC 9457 `problem+json`
with `error_code`, `trace_id`, `request_id`, and structured details (allowlisted
for public safety).

**Forbidden:**

- `anyhow` in library crates (binaries only)
- `Box<dyn Error>` in public return types
- `String`-as-error in new public APIs
- `unwrap` / `expect` in non-test code without explicit safety comment
- `Other(String)` catch-all variants
- Internal state, credentials, or PII in `Display` / `Debug` on error types
- `DebugInfo` detail type in public API responses

**Required at API boundary:**

- Every `ApiError` variant maps to a stable HTTP status and `ErrorCode`
- Every response includes `trace_id` for observability correlation (spec 18)
- Panics are converted to `ActionError::Fatal` or `RuntimeError::ActionPanicked`
  with sanitized message; full trace only in internal logs

**Two-tier projection:**

- Internal logs capture full fidelity (context chain, source chain, detail types including `DebugInfo`)
- Public API responses use only allowlisted detail types and sanitized messages
```

## Open questions

- **`#[derive(Classify)]` macro capabilities** — does it support per-variant attributes as shown in examples? If not, manual `impl` is always available; macro improvement is a v1.x task
- **Client SDK generation** — TS/Python/Rust client libraries generated from OpenAPI spec; error code catalog exported as constants. Deferred to v1.5 or v2
- **Error telemetry cardinality** — «error code» label on metrics is ~50 values; fine. «error category» is 14; fine. Any per-execution label would explode — already forbidden by allowlist (spec 18)
- **Error analytics dashboard** — Grafana panel showing top error codes, per-tenant error rates, retryable vs permanent breakdown. Ship as default dashboard with spec 18
- **Error message localisation** — titles and details in user's locale. Deferred — English only in v1
- **Auto-generated docs pages** — `docs/errors/WORKFLOW_NOT_FOUND.md` for every code, with examples, causes, solutions. Generate from source? Hand-write? Deferred to dedicated docs task
