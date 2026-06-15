# nebula-error — Agent orientation
> Agent quick-map for `crates/error/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Workspace-wide error taxonomy — the `Classify` trait, `NebulaError<E>` wrapper with extensible typed details + context chain, and `RetryHint` so transient-vs-permanent is an explicit decision, not folklore.
**Layer:** Cross-cutting — depends only downward (root AGENTS.md → Layered Dependency Map); has no nebula deps, importable from any tier.

## Commands
- `cargo check -p nebula-error`
- `cargo nextest run -p nebula-error`  ·  doctests: `cargo test -p nebula-error --doc`
- `cargo check -p nebula-error --all-features` — exercise `serde` + `derive` (`#[derive(Classify)]` from sibling `nebula-error-macros`); both off by default

## Key files
- `src/lib.rs` — public re-exports + `Result<T, E>` alias (`= Result<T, NebulaError<E>>`); module gate
- `src/traits.rs` — `Classify` / `ErrorClassifier` — the L2-§12.4 seam every error type implements
- `src/error.rs` — `NebulaError<E>` wrapper; `Display` must emit the full context chain (regression-fixed, do not regress)
- `src/category.rs`, `src/severity.rs`, `src/code.rs` — `ErrorCategory` / `ErrorSeverity` / `ErrorCode` + `codes`
- `src/retry.rs` — `RetryHint` data consumed by `nebula-resilience`
- `src/details.rs`, `src/detail_types.rs` — TypeId-keyed `ErrorDetails` + prebuilt detail structs (`BadRequest`, `FieldViolation`, …)
- `src/collection.rs` — `ErrorCollection` / `BatchResult` aggregation

## Conventions & never-do
- This is the foundation error crate: library crates use `thiserror` + `Classify` + `NebulaError`; `anyhow` is binaries-only (L2-§12.4).
- Stay in your lane: NOT a resilience pipeline (`RetryHint` is data; execution lives in `nebula-resilience`), NOT an API formatter (`nebula-api` maps to RFC 9457 `problem+json`), NOT logging (`nebula-log`).
- `Classify::retry_hint()` is the single transient-vs-permanent decision surface — do not re-implement classification per crate.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design (frontmatter canon-invariant `L2-12.4`)
- Canon: `docs/PRODUCT_CANON.md` §3.10, §4.2 (ErrorClassifier), §12.4 · `docs/MATURITY.md` row `nebula-error`
