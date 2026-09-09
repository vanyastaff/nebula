# nebula-error — Agent orientation
> Local guide for `crates/error/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Workspace-wide error taxonomy — the `Classify` trait, `NebulaError<E>` wrapper with extensible typed details + context chain, and `RetryHint` so transient-vs-permanent is an explicit decision, not folklore.
**Layer:** Cross-cutting — no product-crate dependencies; its optional `nebula-error-macros` companion supplies the derive. Importable from any tier.

## Commands

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

- Stay in your lane: NOT a resilience pipeline (`RetryHint` is data; execution lives in `nebula-resilience`), NOT an API formatter (`nebula-api` maps to RFC 9457 `problem+json`), NOT logging (`nebula-log`).
- `Classify::retry_hint()` is the single transient-vs-permanent decision surface — do not re-implement classification per crate.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Classification derive or serialization | [derive](tests/derive.rs) with `--features derive`; [serde](tests/serde.rs) with `--features serde`. A default build alone does not exercise these features. |
| Display and context chains | Unit tests in [src/error.rs](src/error.rs); preserve context order and caller-owned error details. |

## See also

- `README.md` — full design (frontmatter canon-invariant `L2-12.4`)
- Canon: [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.10, §4.2 (ErrorClassifier), §12.4 · [docs/MATURITY.md](../../docs/MATURITY.md) row `nebula-error`
