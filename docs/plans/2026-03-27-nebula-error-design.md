# nebula-error — Enterprise Error Infrastructure

**Date:** 2026-03-27
**Status:** Approved design, ready for implementation

## Motivation

26 crates in the Nebula workspace each reinvent error classification independently:
- `is_retryable()` duplicated across 6+ crates with different logic
- `NotFound` variant in 6 crates, `ValidationError` in 6 crates
- `error_code()` returns `&'static str` in 6 crates, no unified approach
- `ErrorSeverity` exists only in nebula-validator
- No shared traits — each crate writes its own classification from scratch

Plugin developers have no standard way to classify their errors. The engine has no
uniform way to decide retry/skip/stop based on error metadata.

## Design Principles

- **Google's error model** as foundation: minimal envelope + extensible typed details
- **AWS SDK pattern** for Rust adaptation: generic wrapper `NebulaError<E>` + `Classify` trait
- **Cross-cutting crate**: imported by all layers, minimal dependencies
- **Domain errors stay in their crates**: nebula-error provides classification, not concrete errors

## Architecture

```
nebula-error/                       nebula-error-macros/
  src/
  ├── lib.rs            re-exports    src/
  ├── severity.rs       ErrorSeverity   └── lib.rs  #[derive(Classify)]
  ├── category.rs       ErrorCategory
  ├── code.rs           ErrorCode + codes::*
  ├── retry.rs          RetryHint
  ├── details.rs        ErrorDetails (TypeId-keyed map)
  ├── detail_types.rs   RetryInfo, ResourceInfo, BadRequest, etc.
  ├── traits.rs         Classify trait
  ├── error.rs          NebulaError<E> wrapper
  ├── collection.rs     ErrorCollection<E>
  └── convert.rs        From/Into bridges
```

## Core Types

### ErrorSeverity

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorSeverity {
    Error,    // operation failed
    Warning,  // completed with issues
    Info,     // not a failure, worth logging
}
```

### ErrorCategory

Inspired by `google.rpc.Code` — canonical classification of "what happened":

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCategory {
    NotFound,
    Validation,
    Authentication,
    Authorization,
    Conflict,
    RateLimit,
    Timeout,
    Exhausted,
    Cancelled,
    Internal,
    External,
    Unsupported,
}
```

Extensible via `#[non_exhaustive]` — new categories added as needed.

### ErrorCode

Machine-readable error code. Newtype on `Cow<'static, str>`:

```rust
pub struct ErrorCode(Cow<'static, str>);

impl ErrorCode {
    pub const fn new(code: &'static str) -> Self;
    pub fn custom(code: impl Into<String>) -> Self;
}

pub mod codes {
    pub const NOT_FOUND: ErrorCode = ErrorCode::new("NOT_FOUND");
    pub const VALIDATION: ErrorCode = ErrorCode::new("VALIDATION");
    pub const TIMEOUT: ErrorCode = ErrorCode::new("TIMEOUT");
    // ... canonical codes matching ErrorCategory
}
```

Static codes for standard cases, runtime codes for plugin-specific errors
(e.g., `ErrorCode::custom("STRIPE_CARD_DECLINED")`).

### RetryHint

```rust
pub struct RetryHint {
    pub after: Option<Duration>,
    pub max_attempts: Option<u32>,
}
```

A *hint*, not a command. The resilience layer decides actual retry strategy.

## Classify Trait

Single trait combining AWS `ProvideErrorMetadata` + `ProvideErrorKind`:

```rust
pub trait Classify {
    fn category(&self) -> ErrorCategory;          // required
    fn error_code(&self) -> ErrorCode;            // required

    fn severity(&self) -> ErrorSeverity {         // default: Error
        ErrorSeverity::Error
    }

    fn is_retryable(&self) -> bool {              // default: derived from category
        matches!(self.category(),
            ErrorCategory::Timeout | ErrorCategory::Exhausted | ErrorCategory::External)
    }

    fn retry_hint(&self) -> Option<RetryHint> {   // default: None
        None
    }
}
```

2 required methods, 3 optional with sensible defaults. Minimal barrier for plugin developers.

## ErrorDetails — Google's Any Pattern in Rust

Type-safe extensible storage inspired by `google.rpc.Status.details`:

```rust
pub trait ErrorDetail: Any + Send + Sync + Debug {}

pub struct ErrorDetails { /* HashMap<TypeId, Box<dyn Any + Send + Sync>> */ }

impl ErrorDetails {
    pub fn insert<T: ErrorDetail>(&mut self, detail: T);
    pub fn get<T: ErrorDetail>(&self) -> Option<&T>;
    pub fn has<T: ErrorDetail>(&self) -> bool;
}
```

### Standard Detail Types

Shipped with the crate, inspired by `google.rpc.*` messages:

| Type | Google equivalent | Purpose |
|------|-------------------|---------|
| `RetryInfo` | `google.rpc.RetryInfo` | retry_delay, max_attempts |
| `ResourceInfo` | `google.rpc.ResourceInfo` | resource_type, resource_name, owner |
| `BadRequest` | `google.rpc.BadRequest` | field violations list |
| `FieldViolation` | `google.rpc.BadRequest.FieldViolation` | field, description, code |
| `DebugInfo` | `google.rpc.DebugInfo` | detail string, stack entries |
| `QuotaInfo` | `google.rpc.QuotaFailure` | metric, limit, used |
| `PreconditionFailure` | `google.rpc.PreconditionFailure` | type, subject, description |

Plugin developers can create custom detail types by implementing `ErrorDetail`.

## NebulaError<E> Wrapper

Generic wrapper inspired by AWS `SdkError<E>`:

```rust
pub struct NebulaError<E: Classify> {
    inner: E,                              // domain error
    message: Option<Cow<'static, str>>,    // override message
    details: ErrorDetails,                 // typed details (google Any pattern)
    context: Vec<Cow<'static, str>>,       // context chain (anyhow-style)
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl<E: Classify> NebulaError<E> {
    // Construction
    pub fn new(inner: E) -> Self;

    // Builder
    pub fn with_message(self, msg: impl Into<Cow<'static, str>>) -> Self;
    pub fn with_source(self, source: impl Error + Send + Sync + 'static) -> Self;
    pub fn with_detail<D: ErrorDetail>(self, detail: D) -> Self;
    pub fn context(self, ctx: impl Into<Cow<'static, str>>) -> Self;

    // Accessors — delegate to Classify
    pub fn category(&self) -> ErrorCategory;
    pub fn severity(&self) -> ErrorSeverity;
    pub fn error_code(&self) -> ErrorCode;
    pub fn is_retryable(&self) -> bool;
    pub fn retry_hint(&self) -> Option<RetryHint>;

    // Domain error access
    pub fn inner(&self) -> &E;
    pub fn into_inner(self) -> E;

    // Details
    pub fn detail<D: ErrorDetail>(&self) -> Option<&D>;
    pub fn details(&self) -> &ErrorDetails;
}

impl<E: Classify> From<E> for NebulaError<E> { ... }
```

## ErrorCollection

For batch operations and validation (collect all, don't fail fast):

```rust
pub struct ErrorCollection<E> {
    errors: Vec<NebulaError<E>>,
}

impl<E: Classify> ErrorCollection<E> {
    pub fn push(&mut self, error: NebulaError<E>);
    pub fn any_retryable(&self) -> bool;
    pub fn max_severity(&self) -> ErrorSeverity;
    pub fn uniform_category(&self) -> Option<ErrorCategory>;
}

pub type Result<T, E> = std::result::Result<T, NebulaError<E>>;
pub type BatchResult<T, E> = std::result::Result<T, ErrorCollection<E>>;
```

## Derive Macro

`#[derive(Classify)]` auto-implements the trait from attributes:

```rust
#[derive(Debug, thiserror::Error, Classify)]
pub enum MyError {
    #[classify(category = "timeout", code = "MY_TIMEOUT")]
    #[error("connection timed out")]
    Timeout,

    #[classify(category = "validation", code = "MY_INVALID", severity = "warning")]
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[classify(category = "rate_limit", code = "MY_RATE_LIMIT", retry_after_secs = 60)]
    #[error("rate limited")]
    RateLimited,

    #[classify(category = "external", code = "MY_API_ERR", retryable = false)]
    #[error("API error: {0}")]
    ApiError(String),
}
```

### Attribute reference

| Attribute | Required | Example | Description |
|-----------|:--------:|---------|-------------|
| `category` | yes | `"timeout"` | Maps to `ErrorCategory` variant |
| `code` | yes | `"MY_TIMEOUT"` | Machine-readable error code |
| `severity` | no | `"warning"` | Default: `"error"` |
| `retryable` | no | `false` | Override default retryability |
| `retry_after_secs` | no | `60` | Generates `RetryHint::after(...)` |

## Dependencies

```toml
[package]
name = "nebula-error"
edition = "2024"
rust-version = "1.93"

[dependencies]
thiserror = "2"

[dependencies.serde]
version = "1"
optional = true
features = ["derive"]

[features]
default = []
serde = ["dep:serde"]
derive = ["dep:nebula-error-macros"]
```

Minimal footprint: `thiserror` + optional `serde`. No `anyhow` (we build our own
context chain). No `tracing` (integration via nebula-log, not coupled here).

## What Does NOT Belong Here

- Domain-specific error enums (`ActionError`, `ResourceError`) — stay in their crates
- HTTP/API response mapping — nebula-api's concern
- Logging/tracing integration — nebula-log's concern
- Scope/context types — nebula-core's concern

## Migration Strategy

**Phase 1:** Create nebula-error + nebula-error-macros. Non-breaking.
**Phase 2:** Each crate implements `Classify` on existing errors. Non-breaking.
**Phase 3:** Deprecate duplicate `is_retryable()`, `error_code()` impls. Major bump.

## Implementation Plan

### Phase 1: Core types (no macro)
1. Create crate scaffold (Cargo.toml, lib.rs)
2. `ErrorSeverity`, `ErrorCategory`, `ErrorCode` + `codes::*`
3. `RetryHint`
4. `ErrorDetails` + `ErrorDetail` trait + standard detail types
5. `Classify` trait
6. `NebulaError<E>` wrapper
7. `ErrorCollection<E>` + type aliases
8. Tests for all types
9. Serde feature flag implementations

### Phase 2: Derive macro
10. `nebula-error-macros` crate scaffold
11. Parse `#[classify(...)]` attributes
12. Generate `Classify` impl
13. Tests for derive macro
14. Integration tests (derive + wrapper + details)

### Phase 3: Migration (separate PRs)
15. nebula-resource: implement Classify on Error
16. nebula-action: implement Classify on ActionError
17. nebula-core: implement Classify on CoreError
18. Other crates as needed
