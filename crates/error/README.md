# nebula-error

Enterprise error infrastructure for the Nebula workflow engine — classification traits, generic error wrapper, extensible typed details.

**Layer:** Cross-cutting (depended on by every other crate)
**Canon:** §3.10 (cross-cutting vocabulary), §12.4 (errors and contracts — library crates use typed errors, not `anyhow`)

## Status

**Overall:** `implemented` — the authoritative error taxonomy for the workspace.

**Works today:**

- `Classify` trait — core classification surface: category, code, severity, retryability
- `ErrorClassifier` pattern — makes transient/permanent an explicit decision instead of folklore (canon §4.2)
- `NebulaError<E>` — generic wrapper adding details + context chain (see closed bug #405 — `Display` now includes full context chain)
- `ErrorDetails` / `ErrorDetail` — TypeId-keyed extensible detail storage (Google / AWS SDK inspired)
- `ErrorCategory` — canonical "what happened" classification
- `ErrorSeverity` — `Error` / `Warning` / `Info`
- `ErrorCode` + `codes` — machine-readable code newtype
- `ErrorCollection` / `BatchResult` — aggregation for batch/validation errors
- `RetryHint` — structured retry guidance returned by `Classify`
- `Result<T, E>` type alias using `NebulaError<E>`
- `#[derive(Classify)]` via `nebula-error-macros`
- 10 unit test markers, 2 integration tests

**Known gaps / deferred:**

- None significant. The error taxonomy is stable and in active use across the workspace.
- Historical issue: `Display` used to omit the context chain — fixed in commit `0f047d32` (#405). Referenced here so future authors don't re-introduce the regression.

## Architecture notes

- **Clean module layout.** One file per concept: `category`, `code`, `collection`, `convert`, `detail_types`, `details`, `error`, `retry`, `severity`, `traits`. Eleven modules for 3111 lines — each file is modest.
- **Proc-macros in a separate sibling crate** (`nebula-error-macros`) — correct Rust practice. Keeps compile surface separate.
- **Single intra-workspace dependency** (the macros crate). Correct for a foundational cross-cutting crate.
- **No dead code or compat shims.**
- **DRY caveat:** `detail_types.rs` provides a library of pre-built `ErrorDetail` implementations; if it starts mirroring what domain crates already define, review for whether the detail type belongs closer to its producer.

## What this crate provides

| Type / trait | Role |
| --- | --- |
| `Classify` | Core trait — category, code, severity, retryability. |
| `NebulaError<E>` | Generic wrapper with details + context chain. |
| `ErrorDetails`, `ErrorDetail` | TypeId-keyed extensible detail storage. |
| `ErrorCategory` | Canonical classification. |
| `ErrorSeverity` | Error / Warning / Info. |
| `ErrorCode`, `codes` | Machine-readable code newtype. |
| `ErrorCollection`, `BatchResult` | Batch/validation aggregation. |
| `RetryHint` | Structured retry guidance. |
| `ErrorClassifier` | The pattern of using `Classify` at decision points. |
| `#[derive(Classify)]` | Proc-macro from `nebula-error-macros`. |
| `Result<T, E>` | Alias for `std::result::Result<T, NebulaError<E>>`. |

## Where the contract lives

- Source: `src/lib.rs`, `src/error.rs`, `src/traits.rs`, `src/category.rs`, `src/details.rs`
- Canon: `docs/PRODUCT_CANON.md` §3.10, §12.4
- Glossary: `docs/GLOSSARY.md` §6 (errors)

## See also

- `nebula-error-macros` — sibling proc-macro crate
- `nebula-resilience` — consumes `RetryHint` for retry policy composition
- `nebula-api` — maps `NebulaError` to RFC 9457 `problem+json` at the API boundary
