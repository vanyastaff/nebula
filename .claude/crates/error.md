# nebula-error

Enterprise error infrastructure. Google error model (Status + typed details) adapted to Rust with AWS SDK wrapper pattern.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `Classify` trait: 2 required (`category`, `error_code`), 3 optional with defaults
- `is_retryable()` default from `ErrorCategory`: Timeout, Exhausted, External = retryable
- `ErrorDetails` keyed by TypeId — one value per type, insert overwrites
- `ErrorCode` uses `Cow<'static, str>` — static for canonical, owned for plugin runtime codes
- `ErrorSeverity` ordering: Info < Warning < Error (derives Ord), `#[non_exhaustive]`
- `NebulaError<E>` requires `E: Classify` — classification delegated to domain error
- `NebulaError<E>` implements `Classify` by delegating to inner — usable anywhere `impl Classify` expected
- Serde behind feature flag — not forced on all consumers
- Derive macro behind `derive` feature flag

## Traps

- `ErrorCategory` and `ErrorSeverity` are `#[non_exhaustive]` — match arms need wildcard
- `RetryHint` is advisory — resilience layer may ignore it
- `ErrorDetails::insert` overwrites same-type entry silently (no merge)
- Derive macro panics at compile time for unknown category/severity strings
- `NebulaError<E>` requires `E: Classify + Debug + Display` for full Error trait impl

## Relations

- Depends on: serde (optional), nebula-error-macros (optional); thiserror in dev-dependencies only
- Depended on by: all 21 crates (Classify migration complete 2026-03-30)

<!-- reviewed: 2026-03-30 -->
