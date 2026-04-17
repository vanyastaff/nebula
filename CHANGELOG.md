# Changelog

All notable changes to the Nebula workflow engine are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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
