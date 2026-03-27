# nebula-error

Enterprise error infrastructure for the Nebula workflow engine.

## Layer

Foundation — no upward dependencies. Imported by all crates that need typed errors.

## Key Design Decisions

- **`Cow<'static, str>` for ErrorCode** — most codes are compile-time constants (`&'static str`), but plugins can create runtime codes via `ErrorCode::custom()`. `Cow` avoids allocation for the common case.
- **ErrorSeverity is ordered** — `Info < Warning < Error` so `max()` picks the worst severity in a collection. Uses discriminant values 0/1/2.
- **ErrorCategory is `#[non_exhaustive]`** — new categories may be added. ErrorSeverity intentionally is not (closed set of 3).
- **Serde behind feature flag** — manual impls serialize as lowercase/snake_case strings, not Rust enum variant names.
- **Classify trait has default impls** — only `category()` and `code()` are required; severity defaults to Error, retryability delegates to category.

## Invariants

- `ErrorCode::new()` is `const fn` — canonical codes in `codes` module are true constants.
- Serde round-trip: deserialize(serialize(x)) == x for all types.
- `is_client_error()` and `is_server_error()` are not exhaustive over all categories (RateLimit and Cancelled are neither).

## Traps

- `ErrorCategory::RateLimit` is NOT default-retryable (unlike Timeout/Exhausted/External) — rate limiting needs backoff logic, not blind retry.
- RetryHint serde uses `after_ms` (milliseconds as u64), not Duration's default serde.

<!-- reviewed: 2026-03-26 -->
