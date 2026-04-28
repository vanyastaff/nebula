# Changelog

All notable changes to `nebula-schema` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The 2026-04-28 quality-fixes pass (`refactor(schema)!:` + Phase 2-4 commits)
covers the full set of issues raised in the nebula-schema code review.

### ⚠ Breaking Changes

- **`ExpressionContext::evaluate` signature changed (T01).**  The trait no
  longer uses `#[async_trait::async_trait]`; impls now write
  ```rust
  fn evaluate<'a>(&'a self, ast: &'a ExpressionAst) -> EvalFuture<'a> {
      Box::pin(async move { ... })
  }
  ```
  where `EvalFuture<'a>` is a new public type alias for the boxed future.
  Drops the `async-trait` runtime dependency on the schema crate.

- **`Field::*::new` requires a pre-validated `FieldKey` (T02).**  Static
  keys: `Field::string(field_key!("name"))`.  Runtime keys: `Field::try_string(s)?`
  (new fallible alias, returns `ValidationError` instead of panicking).
  Same change applied to all 13 `Field::*` constructors plus the `FieldCollector`
  closure-DSL methods (`.string()`, `.secret()`, `.number()`, …) and the
  `ObjectBuilder::new` / `ListBuilder::new` constructors.  `~217` call sites
  migrated workspace-wide.

- **`FieldValues::set_raw` removed (T03).**  Use `try_set_raw(...)?` for
  fallible runtime input or `try_set_raw(...).expect("...")` in tests /
  migrations with literal keys.

- **`Loader<T>: PartialEq` removed (T05).**  The previous `always-true` impl
  violated the `PartialEq` contract.  Loaders are not value-comparable; if
  identity comparison is required, use `Arc::ptr_eq` on the inner handle.

- **`KdfParams::Argon2id` gained a new `output_bytes` field (T06).**  The
  variant is `#[non_exhaustive]`, so wire JSON without the field continues
  to deserialize (default `32`).  In-code constructors must add
  `output_bytes: None` (or `Some(n)`).

### Added

- **KDF guardrails (T06).**  Public consts `MIN_KDF_*` / `MAX_KDF_*` /
  `DEFAULT_KDF_OUTPUT_BYTES` for Argon2id memory, time, parallelism, salt
  length, and output length, aligned to RFC 9106 §4 ("Recommended values"
  for interactive use).  `KdfParams::hash_password` now rejects sub-minimum
  costs in addition to over-maximum ones.

- **`recursion_limit` STANDARD_CODE (T07).**  `FieldValues::from_json` and
  `try_set_raw` reject deeply-nested user JSON with the new
  `recursion_limit` code (`MAX_VALUE_DEPTH = 64`).  Closes a stack-overflow
  vector against adversarial wire payloads.

- **`secret.default_forbidden` STANDARD_CODE (T18).**  Lint pass now
  hard-rejects `Field::Secret { default: Some(_) }`.  Symmetric with
  `SecretString::Deserialize` (which always errors) — secrets must
  originate from the resolve pipeline, never from wire JSON.

- **`audit-secret-expose` Cargo feature (T11).**  Off by default;
  `SecretString::expose` / `SecretBytes::expose` log at `tracing::trace!`
  by default (with `#[track_caller]` location), escalating to
  `tracing::debug!` when the feature is enabled.  Lets compliance/audit
  builds opt in to a per-call audit trail without flooding default logs.

- **`tracing::instrument` spans on every hot-path entry point (T10).**
  Covers `ValidSchema::validate`, `ValidValues::resolve`,
  `ValidSchema::json_schema`, `LoaderRegistry::load_options` /
  `load_records`, `lint::lint_tree`, `KdfParams::hash_password`.
  All emit structured fields (field counts, mode flags, error counts).

- **Per-crate `clippy.toml` (T17).**  `crates/schema/clippy.toml` bans
  `async_trait::async_trait` for this crate; `#![deny(clippy::disallowed_macros)]`
  in `lib.rs` escalates to error.  Other crates inherit the workspace
  default.

- **Compile-time `#[derive(Schema)]` conflict detection (T04).**  Three
  new compile-fail tests catch known invalid attribute combinations at
  expansion time instead of runtime: `secret + default`,
  `secret + multiline`, `no_expression + expression_required`.  Generated
  code also emits a `tracing::error!` event with the structured
  `ValidationReport` before any remaining runtime panic so failures are
  visible in logs even when the panic is caught.

### Fixed

- **`first_duplicate_index` no longer falls back to a Debug-formatted
  bucket key (T13).**  `serde_json::Value::to_string` is infallible for
  valid JSON; the previous fallback could produce false-positive
  `items.unique` reports for values whose Debug shapes happened to match.

- **`ExpressionMode` JSON Schema export is symmetric (T14).**  Every
  property now carries `x-nebula-resolved-value-schema` regardless of
  Forbidden / Allowed / Required mode.  UI consumers no longer need to
  branch on mode to find the post-resolution value schema.

- **`FieldKey` "mode"/"value" interned via `LazyLock` (T08).**  Removes ~6
  per-call `FieldKey::new("mode" / "value").expect(..)` allocations on
  every validate / resolve / promote-secrets recursion through a
  `Field::Mode` variant.

- **`SecretBytes::Drop` no longer double-zeroizes (T09).**  `Zeroizing<Vec<u8>>`
  already zeroes the heap buffer in its blanket Drop; the manual impl
  was redundant.

- **`resolve()` skips post-resolve revalidate when nothing changed (T12).**
  Schemas with no expression-bearing fields no longer pay the cost of
  a second full schema walk.  ~50 % wall-clock saving on the static-only
  fast path.

- **Doc rot.**  README pointers to deleted `docs/INTEGRATION_MODEL.md`,
  `docs/MATURITY.md`, `docs/PRODUCT_CANON.md`, the `docs/adr/0001-..0003-...`
  set, and `docs/adr/0034-..` (in `secret.rs`) replaced with inline
  pointers and references to the workspace `ARCHITECTURE.md`.
  `cargo doc -p nebula-schema --no-deps` is now warning-free.

### Deferred

- **T15 — moving `crates/schema/examples/` into a root-level `examples/`
  workspace member.**  Discovery during execution surfaced ~10 sibling
  crates with the same per-crate `examples/` shape; this is a
  workspace-wide migration that warrants its own plan rather than being
  done piecemeal in a `nebula-schema` PR.
