# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- **Ground truth:** `src/parameter.rs` (Parameter struct), `src/parameter_type.rs` (ParameterType enum), `src/collection.rs` (ParameterCollection).
- `ValidatedValues` only constructible via `ParameterCollection::validate()` or `ValidationReport::into_validated()` — `pub(crate)` constructor.
- `ValidationReport` carries values internally — `into_validated()` takes no args. Not `Default`; only constructible via `ValidationReport::new()` (pub(crate)).
- Loaders are fallible — return `Result<LoaderResult<T>, LoaderError>`.
- `Condition` (visibility/required predicate) is separate from `Rule` (value constraint). Never mix.

## Key Decisions
- v3 API: `Parameter` struct + `ParameterType` enum (19 variants). Shared metadata on struct; type-specific in enum.
- `ParameterCollection` replaces old `Schema`. `ParameterValues` replaces old `FieldValues` (aliases removed).
- `ParameterCollection` implements `FromIterator`, `IntoIterator`, `extend()`, and `iter()`.
- `ValidatedValues` delegates typed accessors directly — no `.raw()` needed for common access.
- `Condition` shorthand constructors for all 11 variants: `eq`, `ne`, `one_of`, `set`, `not_set`, `is_true`, `gt`, `lt`, `all`, `any`, `not`.
- `Condition::one_of` accepts `IntoIterator<Item: Into<Value>>` — no `json!()` wrappers needed for strings/ints.
- `lint_collection`, `LintDiagnostic`, `LintLevel` re-exported in prelude.
- `ModeVariant` removed — Mode variants are `Vec<Parameter>` (param.id = variant key).
- `DisplayMode` controls Object: Inline, Collapsed, PickFields, Sections. PickFields/Sections skip backfill for absent keys.
- `Transformer` applied lazily via `get_transformed()` — does NOT affect validation/normalization.
- `FieldSpec` restricted subset (4 variants) for dynamic providers. Round-trips via `TryFrom`/`From`.

## Traps
- `docs/crates/parameter/` describes removed v1 APIs — do not use.
- `Rule` re-exported from `nebula_validator` — same type, one source.
- Type-specific builders `debug_assert!` on wrong `ParameterType` variant (panics in debug, zero cost in release).
- Unknown fields inside nested objects produce **warnings**, not errors (even in Strict profile).
- `ParameterError::ValidationError` removed — use `ValidationIssue`.

## Relations
- Used by nebula-action (re-exports `Parameter`, `ParameterCollection`), nebula-credential, nebula-sdk, nebula-macros.
- `Rule::field_references()` from nebula-validator used for lint cross-referencing.

<!-- reviewed: 2026-04-01 — Parameters derive moved to nebula-parameter-macros, re-exported from crate root -->
<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 -->
