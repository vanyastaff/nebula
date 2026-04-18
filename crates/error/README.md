---
name: nebula-error
role: Error Taxonomy and Classification Boundary
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-12.4]
related: [nebula-resilience, nebula-core, nebula-api]
---

# nebula-error

## Purpose

Rust crates that each define their own error enum and retry logic end up with incompatible
classification strategies — one crate treats `TIMEOUT` as retryable, another treats the same
timeout as terminal. `nebula-error` solves this by providing a single workspace-wide error
taxonomy: a `Classify` trait that every error type implements, a `NebulaError<E>` generic wrapper
that adds extensible details and a context chain, and structured retry guidance through `RetryHint`.
This makes transient vs permanent failure an explicit decision rather than folklore scattered across
individual action implementations.

## Role

**Error Taxonomy and Classification Boundary** — the foundation error crate that every library
crate in the workspace depends on. Pattern: *ErrorClassifier* (canon §4.2, `docs/GLOSSARY.md` §6).
Every library crate uses `thiserror` + `Classify` + `NebulaError`; only binaries use `anyhow`.
`nebula-api` maps `NebulaError` to RFC 9457 `problem+json` at the API boundary.

## Public API

- `Classify` — core trait: `category()`, `code()`, `severity()`, `retry_hint()`.
- `ErrorClassifier` — the `Classify`-at-decision-points pattern (canon §4.2).
- `NebulaError<E>` — generic wrapper over any `Classify` type; adds `ErrorDetails` chain and context.
- `ErrorDetails`, `ErrorDetail` — TypeId-keyed extensible detail storage (Google / AWS SDK style).
- `ErrorCategory` — canonical "what happened" classification (`Transient`, `Permanent`, `Internal`, …).
- `ErrorSeverity` — `Error` / `Warning` / `Info`.
- `ErrorCode`, `codes` — machine-readable code newtype.
- `ErrorCollection`, `BatchResult` — aggregation for batch and validation errors.
- `RetryHint` — structured retry guidance returned by `Classify::retry_hint()`.
- `Result<T, E>` — alias for `std::result::Result<T, NebulaError<E>>`.
- `#[derive(Classify)]` — proc-macro from `nebula-error-macros` (feature `derive`).
- Pre-built detail types: `BadRequest`, `FieldViolation`, `ResourceInfo`, `RequestInfo`, `ExecutionContext`, `DebugInfo`, and others in `detail_types`.

## Contract

- **[L2-§12.4]** All library crates in the workspace use typed errors (`thiserror` + `Classify` + `NebulaError`), not `anyhow`. `anyhow` is reserved for binaries. Seam: `crates/error/src/traits.rs` — `Classify`. Test coverage: see `docs/MATURITY.md`.
- **[L3-§12.4]** `Display` for `NebulaError<E>` must include the full context chain (regression: fixed in commit `0f047d32`, #405 — do not regress).
- **[L1-§4.2]** `Classify::retry_hint()` is the explicit decision surface for transient vs permanent failure — `nebula-resilience` consumes `RetryHint` to compose retry policies without re-implementing classification in each crate.

## Non-goals

- Not a resilience pipeline — `RetryHint` is data; the actual retry execution lives in `nebula-resilience`.
- Not an API error formatter — `nebula-api` maps `NebulaError` to RFC 9457 `problem+json`.
- Not a logging system — error display and structured logging are handled by `nebula-log`.

## Maturity

See `docs/MATURITY.md` row for `nebula-error`.

- API stability: `stable` — `Classify`, `NebulaError`, `ErrorCategory`, and `RetryHint` are in active use across the full workspace; no known planned breaking changes.
- `detail_types` module may grow new pre-built detail structs as domain crates identify common error shapes.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.10 (cross-cutting vocabulary), §4.2 (ErrorClassifier pattern), §12.4 (errors and contracts).
- Glossary: `docs/GLOSSARY.md` §6 (errors: `NebulaError`, `Classify`, `ErrorClassifier`, `ApiError`).
- Siblings: `nebula-error-macros` (sibling proc-macro crate), `nebula-resilience` (consumes `RetryHint`), `nebula-api` (maps to RFC 9457).
