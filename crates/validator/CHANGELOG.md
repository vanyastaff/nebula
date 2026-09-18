# Changelog

All notable changes to `nebula-validator` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Compatibility obligations for the declarative rule surface — error codes, field-path
format, and the `Rule` wire encoding — are catalogued in
[`docs/migration.md`](docs/migration.md). Error codes are frozen by
`tests/fixtures/compat/error_registry_v1.json`.

## [Unreleased]

### Breaking Changes

- **Nested error trees are capped at `MAX_ERROR_TREE_DEPTH` (64).** `with_nested` and
  `with_nested_error` trim children below the ceiling and record the dropped count as a
  `nested_errors_omitted` param on the node where the cut happened. Previously an
  over-deep tree aborted the process: every traversal (`kind`, `total_error_count`,
  `flatten`, `to_json_value`, `Display`) and the derived `Clone`/`PartialEq`/`Debug`/`Drop`
  impls recurse once per level.
- **`CollectionNested` preserves the element diagnostic.** Element failures keep their
  original `code`, `field`, `params`, `severity`, and nested errors, and gain an `index`
  param. Fail-fast mode wraps the element error as a nested child. Previously the
  diagnostic was rebuilt with `ValidationError::new`, discarding all of that.
- **`PredicateContext::is_empty` now agrees with `len()`.** A scalar JSON root is a binding
  addressable as `FieldPath::root()`, so `from_json(json!(42)).len() == 1` and `is_empty()`
  is `false`. Empty containers still bind nothing.
- **Leaf deserialization is bounded.** `ValueRule`, `Predicate`, and `DeferredRule` now
  route `Deserialize` through the bounded `Rule` path, so they enforce the same depth,
  node, operand, JSON-node, JSON-depth, and text budgets. Over-budget wire that previously
  deserialized successfully is now rejected.
- **`WithCode` is a distinct type.** `WithCode::new(validator, "...")` sets the *code*, and
  `WithCode` no longer exposes `WithMessage`'s message-oriented constructor surface.
- **`combinators::prelude` and `foundation::prelude` were removed.** Use
  `nebula_validator::prelude` or import items from their module paths.
- **Removed from the public API:** `ValidationMode::is_collect_all`,
  `ValidationErrors::into_result`, the `ValidationResult` / `ValidationResultMulti` aliases,
  and the previously public `FieldError` wrapper (now crate-private; `Field` continues to
  return a `ValidationError` with a composed `field` path).
- **`ValidationError.field` is private.** The type guarantees the path is a canonical
  RFC 6901 pointer; a public field let safe external code store raw dot notation and emit an
  envelope whose `field` and `pointer` keys disagreed. Read it through `field_pointer()`.
- **`EvaluationOutcome` and `DiagnosticDisclosure` are `#[non_exhaustive]`.** Downstream
  matches need a wildcard arm. `nebula-schema` records an unknown `EvaluationOutcome`
  variant as a pending obligation rather than treating it as satisfied.

### Fixed

- **`named_field` corrupted nested field paths.** Composing a parent name over an inner
  validator that had already recorded a path produced `/profile/~1email` — the two halves
  were joined with a dot and then RFC 6901-escaped, so the separator became part of the key
  name. Parent and child pointers now join as pointer segments (`/profile/email`).
- `Email` and `Url` no longer `unwrap()` a `LazyLock` regex. A pattern that fails to
  compile — a build-time regression guarded by `built_in_patterns_compile` — surfaces as an
  `unavailable` diagnostic instead of a panic.
- Removed `unwrap`/`expect` panic paths from `MultiField`, `CollectJsonFields`, and
  `Rule::all` single-error handling.

### Added

- `MAX_ERROR_TREE_DEPTH` is exported from `nebula_validator::foundation`.
- `ValidationError::rendered_message` is documented and covered by tests.
- `FieldValidateExt::for_field` is documented with a compile-checked example.
- Registry entry and governance coverage for the `multiple_field_errors` code
  (`error_registry_v1.json` version 1.3.0).
- `governance_policy_test::migration_authority_file_exists` checks that the registry's
  migration authority resolves to a real file.

### Documentation

- `docs/DESIGN.md` translated to English and refreshed against the current code.
- Removed documentation for the never-shipped `Cached` combinator and the removed
  `Logic` enum; rule examples rewritten for the typed sum-of-sums API.
- `docs/migration.md` no longer references deleted crates or test files that do not exist.

## [0.1.0] - 2026-05-05

Initial implementation: `Validate`/`ValidateExt`/`Validatable` traits, `AnyValidator`,
`ValidationError`/`ValidationErrors`, the declarative `Rule` arena and its engine, the
visibility/required policy engine, built-in validators, and `#[derive(Validator)]`.
