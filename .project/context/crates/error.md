# nebula-error

Enterprise error infrastructure. Google error model (Status + typed details)
adapted to Rust with AWS SDK wrapper pattern.

## Invariants

- `Classify` trait: 2 required (`category`, `error_code`), 3 optional with defaults. `NebulaError<E>` requires `E: Classify` and delegates classification to the inner type.
- `is_retryable()` default from `ErrorCategory`: Timeout, Exhausted, External, RateLimit, Unavailable = retryable. `DataTooLarge` is **not** default-retryable (client must reduce payload).
- `ErrorDetails` is keyed by `TypeId` — one value per type, `insert` overwrites silently (no merge).
- `ErrorCode` uses `Cow<'static, str>` — static for canonical codes, owned for plugin runtime codes. Implements `PartialEq<&str>` for ergonomic comparisons.
- `ErrorSeverity` is `Info < Warning < Error` (derives `Ord`), `#[non_exhaustive]`. `ErrorCategory` is also `#[non_exhaustive]` — match arms need a wildcard.
- `RetryHint` is both a `Classify` return type **and** an `ErrorDetail` — attachable via `.with_detail()`. Advisory for the resilience layer (backoff floor, not absolute).
- `NebulaError::map_inner()` preserves message, details, context_chain, and source while transforming the inner type.
- Serde and the `derive` macro are behind feature flags. Derive is re-exported as `nebula_error::Classify` (not `DeriveClassify`).

## Categories (14)

NotFound, Validation, Authentication, Authorization, Conflict, RateLimit, Timeout, Exhausted, Cancelled, Internal, External, Unsupported, Unavailable (503, retryable), DataTooLarge (413, client error).

## Detail types (11)

`RetryHint` · `ResourceInfo` · `BadRequest` / `FieldViolation` · `DebugInfo` · `QuotaInfo` · `PreconditionFailure` / `PreconditionViolation` · `ExecutionContext` (node_id, workflow_id, correlation_id, attempt) · `ErrorRoute` (suggested_handler, dead_letter) · `TypeMismatch` (expected, actual, location) · `HelpLink` · `RequestInfo` · `DependencyInfo`.

## HTTP mapping

- `ErrorCategory::http_status_code()` — category → HTTP status, `const fn`.
- `ErrorCategory::from_http_status()` — reverse, **lossy**: 429 → RateLimit, not Exhausted.

## Classification helpers

`ErrorClassifier::new(|cat| ...)` — predicate-based category filtering. Built-ins: `retryable()`, `client_errors()`, `server_errors()`. Used by the resilience layer for conditional retry routing.

## Traps

- `#[derive(Classify)]` panics at compile time on unknown category / severity strings.
- `NebulaError<E>` needs `E: Classify + Debug + Display` for the full `Error` trait.
- `from_http_status(429)` returns `RateLimit`, not `Exhausted` — reverse mapping is lossy.

## Relations

Depends on: `serde` (optional), `nebula-error-macros` (optional); `thiserror` in dev-deps only.
Depended on by: every other crate. 14 of them use `#[derive(Classify)]`; core/credential/action/engine/resilience keep hand-rolled `Classify` impls for cascade risk, active dev, or generic `CallError<E>` reasons.
