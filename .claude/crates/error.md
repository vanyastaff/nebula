# nebula-error

Enterprise error infrastructure. Google error model (Status + typed details) adapted to Rust with AWS SDK wrapper pattern.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `Classify` trait: 2 required (`category`, `error_code`), 3 optional with defaults
- `is_retryable()` default from `ErrorCategory`: Timeout, Exhausted, External, RateLimit = retryable
- `ErrorDetails` keyed by TypeId — one value per type, insert overwrites
- `ErrorCode` uses `Cow<'static, str>` — static for canonical, owned for plugin runtime codes
- `ErrorSeverity` ordering: Info < Warning < Error (derives Ord), `#[non_exhaustive]`
- `NebulaError<E>` requires `E: Classify` — classification delegated to domain error
- `NebulaError<E>` implements `Classify` by delegating to inner — usable anywhere `impl Classify` expected
- `NebulaError::map_inner()` preserves message, details, context_chain, source while transforming inner type
- Serde behind feature flag — not forced on all consumers
- Derive macro behind `derive` feature flag

## Detail Types

- `RetryInfo` — retry delay + max attempts
- `ResourceInfo` — resource type/name/owner
- `BadRequest` / `FieldViolation` — field-level validation errors
- `DebugInfo` — diagnostic detail + stack entries
- `QuotaInfo` — metric/limit/used for quota failures
- `PreconditionFailure` / `PreconditionViolation` — unmet preconditions
- `ExecutionContext` — node_id, workflow_id, correlation_id, attempt (workflow tracing)
- `ErrorRoute` — suggested_handler, dead_letter (error-edge routing)
- `TypeMismatch` — expected, actual, location (DAG edge type validation)

## HTTP Mapping

- `ErrorCategory::http_status_code()` — maps category to HTTP status (const fn)
- `ErrorCategory::from_http_status()` — reverse mapping (429 → RateLimit; lossy for Exhausted)

## ErrorClassifier

- Predicate-based category filtering: `ErrorClassifier::new(|cat| ...)`
- Built-in: `retryable()`, `client_errors()`, `server_errors()`
- Used by resilience layer for conditional retry routing

## Traps

- `ErrorCategory` and `ErrorSeverity` are `#[non_exhaustive]` — match arms need wildcard
- `RetryHint` is advisory — resilience layer may ignore it
- `ErrorDetails::insert` overwrites same-type entry silently (no merge)
- Derive macro panics at compile time for unknown category/severity strings
- `NebulaError<E>` requires `E: Classify + Debug + Display` for full Error trait impl
- `from_http_status(429)` returns `RateLimit`, not `Exhausted` — lossy reverse mapping

## Relations

- Depends on: serde (optional), nebula-error-macros (optional); thiserror in dev-dependencies only
- Depended on by: all 21 crates (Classify migration complete 2026-03-30)

<!-- reviewed: 2026-03-30 -->
