# Changelog

All notable changes to the Nebula workflow engine are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Documentation

- **nebula-sdk** (P11): Credential/OAuth re-export audit — described how the façade exposes `nebula_credential` (full crate + prelude picks), what is intentionally not part of the SDK, and how integrators should migrate when credential types change. See `crates/sdk/README.md` §*Credential, OAuth, and the SDK*.

### Fixed

- **nebula-schema** (PR review): `SecretString` now uses `Zeroizing<String>` so `expose()` stays infallible under `#![forbid(unsafe_code)]`; secret promotion uses `mem::take` + `password.zeroize()` on the KDF path (restore password on KDF error); incompatible secret shapes report a static label (no `Debug` of values); mode payload key construction reports `ValidationError` instead of `expect`. **docs:** correct relative ADR link from `GLOSSARY.md`.

### Added

- **nebula-credential**: `HasCredentialsExt` extension trait with typed `credential::<C>()` and `try_credential::<C>()` methods for ergonomic credential access.
- **nebula-credential**: `RedactedSecret<S>` wrapper for serde-safe redaction of secret values.
- **nebula-core**: `CredentialNotConfigured`, `CredentialNotFound`, `CredentialAccessDenied` error variants on `CoreError`.
- **nebula-schema** (PR-3 C1, Phase 4): `ValidSchema::json_schema()` export behind
  `schemars` feature (Draft 2020-12). Maps core field/value rules
  (`minLength`/`maxLength`, `pattern`, `format`, `minimum`/`maximum`,
  `exclusiveMinimum`/`exclusiveMaximum`, `enum`, `minItems`/`maxItems`) and
  expression wrappers; includes `x-nebula-*` contract extensions for
  `ExpressionMode`, required/visibility modes, root rules, and select/mode/file
  metadata that is not representable in plain JSON Schema.
- **nebula-schema** (Phase 3 security, [ADR-0034](docs/adr/0034-schema-secret-value-credential-seam.md)):
  `SecretValue` / `SecretWire`, optional `KdfParams` + Argon2id on `Field::Secret`,
  `FieldValue::SecretLiteral`, `ResolvedValues::get_secret`, and
  `LoaderContext::with_secrets_redacted`. `ValidValues::resolve` promotes string
  secrets and runs KDF before the final validate pass. Documented in
  `GLOSSARY.md`.
- **nebula-validator**: Re-export `validate_rules_with_ctx` at the crate root (with `validate_rules`) so callers do not need `nebula_validator::engine::` paths.
- **nebula-schema**: `SchemaBuilder::root_rule` and `ValidSchemaInner::root_rules`
  — schema-level rules evaluated after per-field validation (predicate-aware via
  `PredicateContext` from submitted JSON). `#[derive(Schema)]` supports
  `#[schema(custom = "...")]` mapping to `Rule::custom` (deferred wire hook).
  Re-export `Rule` / `Predicate` from `nebula-schema` for macro expansion.
- **nebula-schema**: trybuild `derive_schema_enum_select_vec`; **docs**: `GLOSSARY.md` entries for `enum_select` and `validate_rules_with_ctx`.
- **nebula-schema**: New crate replacing `nebula-parameter`. Implements a
  proof-token validation pipeline (`Schema::builder() → ValidSchema →
  ValidValues → ResolvedValues`) with a unified structured `ValidationError`,
  tree-based `FieldValue`, `field_key!()` compile-time macro, `ExpressionMode`
  per field, consolidated 13-variant `Field` enum, and `InputHint` for String
  fields. See
  `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`.
- **nebula-schema**: `STANDARD_CODES` vocabulary of 36 error codes; 30/36
  covered by integration tests in Phase 1; 6 codes deferred to Phase 4
  (`expression.{parse,runtime,type_mismatch}`, `mode.required`,
  `items.unique`, `loader.{not_registered,failed}`).
- **nebula-schema**: Compile-fail trybuild fixtures: 8 fixtures enforcing
  type-safety contracts (`FieldKey` no-dash, no-empty, `from_str` removed,
  widget mismatches, builder misuse guards).
- **nebula-schema**: O(1) `FieldPath`-indexed field lookup in `ValidSchema`
  (16.5 ns for 100-field schemas vs 73.5 ns linear `find_by_key`).
- **nebula-schema-macros**: `field_key!("name")` proc-macro for compile-time
  `FieldKey` validation; rejects empty strings, leading/trailing whitespace,
  and dot separators at compile time.
- **nebula-validator**: `RuleContext` trait — `Rule::evaluate` now takes
  `&dyn RuleContext` instead of `&HashMap<String, Value>`, eliminating
  per-nesting allocations on nested-object validation descent.

### Changed

- **BREAKING — nebula-engine**: `RefreshCoordinator`, `RefreshAttempt`, and `RefreshConfigError` moved from `nebula-credential` to `nebula-engine::credential::refresh` (breaking import change).
- **BREAKING — nebula-credential**: `CredentialAccessor` trait unified with `nebula-core::CredentialAccessor`; local trait removed.
- **BREAKING — nebula-credential**: `CredentialContext` redesigned to embed `Arc<BaseContext>` from `nebula-core`; `CredentialResolverRef` removed.
- **BREAKING — nebula-credential**: `SecretString` replaced with `secrecy` crate wrapper; `expose_secret()` now returns `&str` directly.
- **nebula-credential**: `#[derive(Credential)]` now generates `DeclaresDependencies` impl; `#[uses_resource(...)]` attribute supported, `#[uses_credential(...)]` emits compile error.
- **nebula-credential** (architecture cleanup): Redistributed credential responsibilities across canonical home crates per [ADR-0028](docs/adr/0028-cross-crate-credential-invariants.md). Rotation orchestration (blue-green, grace period, transaction state machine) moved to `nebula-engine::credential::rotation`; OAuth HTTP flow and token exchange moved to `nebula-api::credential` and `nebula-engine::credential::rotation`; store implementations gated behind `test-util` feature (canonical impls in `nebula-storage`). Flattened `credentials/oauth2/` subdirectory into flat module layout. Added `nebula-eventbus` dependency with `CredentialEventBus` type alias. Target dependency set: `nebula-core`, `nebula-metadata`, `nebula-schema`, `nebula-resilience`, `nebula-eventbus`, `nebula-error`.
- **BREAKING — nebula-schema**: `ResolvedValues::get` no longer returns JSON for
  `Field::Secret` (always `None`); use `ResolvedValues::get_secret` for secret
  material after `resolve`. Default JSON for `FieldValue` encodes
  `SecretLiteral` as `"<redacted>"` (never plaintext in `to_json`).
- **BREAKING — workspace**: MSRV raised from **1.94 → 1.95** (see
  [ADR-0019](docs/adr/0019-msrv-1.95.md); supersedes ADR-0010).
  `workspace.package.rust-version`, `clippy.toml` `msrv`, all
  `.github/workflows/ci.yml` toolchain pins, CLI templates, and docs moved
  together. Contributors must `rustup install 1.95`. Unlocks `if let` guards,
  `cfg_select!`, atomic `update` / `try_update`, and `core::range` for
  follow-up refactors (tracked in ADR-0019 §Follow-ups).
- **nebula-validator**: **Breaking** — replaced flat 30-variant `Rule` enum
  with a typed sum-of-sums: `Rule::{Value(ValueRule), Predicate(Predicate),
  Logic(Box<Logic>), Deferred(DeferredRule), Described(Box<Rule>, String)}`.
  Each kind has a single method that makes sense for it (`validate_value`
  on `ValueRule`, `evaluate` on `Predicate`, etc.). Cross-kind silent-pass
  is gone (calling `validate_value` on a `Predicate` no longer compiles).
  Predicates now carry `FieldPath` instead of raw `String` — paths
  validated at construction. See
  `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`.
- **nebula-validator**: **Breaking wire format** — externally-tagged
  tuple-compact encoding for `Rule`: `{"min_length":3}`, `{"eq":["/path",value]}`,
  `"email"` for unit variants. ~60% shorter than the old `{"rule":"min_length","min":3}`.
  Manual `Deserialize` produces friendly `unknown rule "X". Known rules: ...`
  errors instead of serde's generic "data did not match any variant".
- **nebula-validator**: `Described(Box<Rule>, String)` decorator replaces
  per-variant `message: Option<String>` fields and now works across
  combinators (not just leaf rules). Messages can contain `{placeholder}`
  templates that render from the error's params at `Display` time; zero
  allocation for plain messages.
- **nebula-validator**: `FieldPath` now implements `Serialize`/`Deserialize`
  (wire form is the inner JSON Pointer string).
- **nebula-validator**: `PredicateContext` typed newtype replaces raw
  `HashMap<String, Value>` for predicate evaluation; auto-flattens nested
  JSON objects into `/path` keys.
- **nebula-schema**: consumer updated for new `Rule` shape — `lint.rs`
  classification and `field.rs` builder calls migrated.
- **BREAKING — nebula-action, nebula-credential, nebula-sdk**: Migrated from
  `nebula-parameter` to `nebula-schema`. API mapping:
  `ParameterCollection` → `ValidSchema`, `Parameter::*` variants →
  `Field::*`, `ParameterValues` → `ResolvedValues`/`FieldValues`.
- **BREAKING — Field variants**: `Date`, `DateTime`, `Time`, `Color`,
  `Hidden` removed; replaced by `StringField::hint(InputHint::*)` and
  `VisibilityMode::Never`.
- **BREAKING — FieldKey construction**: `FieldKey::from(&'static str)`
  (panicking) removed. Use `field_key!("name")` for compile-time validation
  or `FieldKey::new(s)?` for runtime.
- **BREAKING — SchemaBuilder**: `Schema::new().add()` replaced by
  `Schema::builder().add(…).build()?`. Build step runs structural lint pass
  (`lint_tree`) and constructs the `FieldHandle` index.

### Removed

- **nebula-credential**: Local `CredentialAccessor` trait (use `nebula_core::CredentialAccessor`).
- **nebula-credential**: `CredentialResolverRef` (use `CredentialContext` with `HasCredentials`).
- **nebula-credential**: `refresh` module removed — types (`RefreshCoordinator`, `RefreshAttempt`, `RefreshConfigError`) now live in `nebula-engine::credential::refresh`.
- **nebula-credential**: Removed duplicate `retry.rs` (use `nebula_resilience` directly). Removed `reqwest`, `futures`, `wiremock` dependencies and the `oauth2-http` feature gate — HTTP transport now lives in `nebula-api` and `nebula-engine`.
- **BREAKING — nebula-parameter** and **nebula-parameter-macros** crates
  deleted from the workspace. Migration complete as of Tasks 28–31.

### Performance

- `schema_validate_static` (legacy API hot-path rewrite): 481 ns → ~79 ns
  (6.1× within Phase 1; **1.54× faster than Phase 0 baseline of 121.87 ns**).
  Note: the ≥2× acceptance target (≤61 ns) is not met on this 3-field flat
  workload, which minimises the RuleContext allocation win. See
  `crates/schema/benches/RESULTS.md` for detailed analysis.
- New `schema_validate_nested` bench (Phase 1 addition): ~872 ns for two
  nested object fields, exercising the `RuleContext` descent path that Phase 0
  could not measure (the old flat `FieldValues` had no nested-object support).
- `resolve_literal_only_fast_path`: ~99 ps (effectively branch-eliminated for
  literal-only schemas — `uses_expressions == false` early return).
- `find_by_path_100_fields`: 16.5 ns (O(1) `IndexMap` index, Task 20).
