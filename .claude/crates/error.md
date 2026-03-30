# nebula-error

Enterprise error infrastructure. Google error model (Status + typed details) adapted to Rust with AWS SDK wrapper pattern.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `Classify` trait: 2 required (`category`, `error_code`), 3 optional with defaults
- `is_retryable()` default from `ErrorCategory`: Timeout, Exhausted, External, RateLimit, Unavailable = retryable
- `ErrorDetails` keyed by TypeId — one value per type, insert overwrites
- `ErrorCode` uses `Cow<'static, str>` — static for canonical, owned for plugin runtime codes
- `ErrorSeverity` ordering: Info < Warning < Error (derives Ord), `#[non_exhaustive]`
- `NebulaError<E>` requires `E: Classify` — classification delegated to domain error
- `NebulaError<E>` implements `Classify` by delegating to inner — usable anywhere `impl Classify` expected
- `NebulaError::map_inner()` preserves message, details, context_chain, source while transforming inner type
- **`RetryHint` is both a Classify return type AND an ErrorDetail** — can be attached to `NebulaError` via `.with_detail()`
- `RetryInfo` removed — use `RetryHint` instead
- `ErrorCode` implements `PartialEq<&str>` — ergonomic comparisons in tests and application code
- Serde behind feature flag — not forced on all consumers
- Derive macro behind `derive` feature flag, re-exported as `nebula_error::Classify` (not `DeriveClassify`)

## Categories (14)

NotFound, Validation, Authentication, Authorization, Conflict, RateLimit, Timeout, Exhausted, Cancelled, Internal, External, Unsupported, **Unavailable** (503, retryable), **DataTooLarge** (413, client error)

## Detail Types (11)

- `RetryHint` — retry delay + max attempts (also returned by `Classify::retry_hint()`)
- `ResourceInfo` — resource type/name/owner
- `BadRequest` / `FieldViolation` — field-level validation errors
- `DebugInfo` — diagnostic detail + stack entries
- `QuotaInfo` — metric/limit/used for quota failures
- `PreconditionFailure` / `PreconditionViolation` — unmet preconditions
- `ExecutionContext` — node_id, workflow_id, correlation_id, attempt (workflow tracing)
- `ErrorRoute` — suggested_handler, dead_letter (error-edge routing)
- `TypeMismatch` — expected, actual, location (DAG edge type validation)
- `HelpLink` — url + description (documentation/troubleshooting links)
- `RequestInfo` — request_id + serving_data (API-layer correlation)
- `DependencyInfo` — service, endpoint, status_code (downstream failure info)

## HTTP Mapping

- `ErrorCategory::http_status_code()` — maps category to HTTP status (const fn)
- `ErrorCategory::from_http_status()` — reverse mapping (429 → RateLimit; lossy for Exhausted)

## ErrorClassifier

- Predicate-based category filtering: `ErrorClassifier::new(|cat| ...)`
- Built-in: `retryable()`, `client_errors()`, `server_errors()`
- Used by resilience layer for conditional retry routing

## Traps

- `ErrorCategory` and `ErrorSeverity` are `#[non_exhaustive]` — match arms need wildcard
- `RetryHint` is advisory — resilience layer uses it as backoff floor, not absolute
- `ErrorDetails::insert` overwrites same-type entry silently (no merge)
- Derive macro panics at compile time for unknown category/severity strings
- `NebulaError<E>` requires `E: Classify + Debug + Display` for full Error trait impl
- `from_http_status(429)` returns `RateLimit`, not `Exhausted` — lossy reverse mapping
- `DataTooLarge` is NOT default-retryable (client must reduce payload size)

## Relations

- Depends on: serde (optional), nebula-error-macros (optional); thiserror in dev-dependencies only
- Depended on by: all 21 crates (Classify migration complete 2026-03-30)
- nebula-memory is the first crate using `#[derive(Classify)]` — reference impl for migrating others

<!-- reviewed: 2026-03-30 (PartialEq<&str>, Classify re-export rename) -->
