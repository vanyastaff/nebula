# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- **Ground truth:** `src/parameter.rs` (Parameter struct), `src/parameter_type.rs` (ParameterType enum), `src/collection.rs` (ParameterCollection).
- `ValidatedValues` only constructible via `ParameterCollection::validate()` — `pub(crate)` constructor.
- Loaders are fallible — return `Result<LoaderResult<T>, LoaderError>`.
- `Condition` (visibility/required predicate) is separate from `Rule` (value constraint). Never mix.

## Key Decisions
- v3 API: `Parameter` struct + `ParameterType` enum (19 variants). Shared metadata on struct; type-specific in enum.
- `ParameterCollection` replaces old `Schema`. `ParameterValues` replaces `FieldValues`.
- `ModeVariant` removed — Mode variants are `Vec<Parameter>` (param.id = variant key).
- `DisplayMode` controls Object: Inline, Collapsed, PickFields, Sections. PickFields/Sections skip backfill for absent keys.
- `Transformer` applied lazily via `get_transformed()` — does NOT affect validation/normalization.
- `Condition` has its own enum (Eq, Ne, OneOf, Set, NotSet, IsTrue, Gt, Lt, All, Any, Not) using `ParameterPath`.
- `FieldSpec` restricted subset (4 variants) for dynamic providers. Round-trips via `TryFrom`/`From`.

## Traps
- `docs/crates/parameter/` describes removed v1 APIs — do not use.
- `Rule` re-exported from `nebula_validator` — same type, one source.
- Type-specific builders silently no-op on wrong `ParameterType` variant.
- Unknown fields inside nested objects produce **warnings**, not errors (even in Strict profile).
- `ParameterError::ValidationError` removed — use `ValidationIssue`.

## Relations
- Used by nebula-action (re-exports `Parameter`, `ParameterCollection`), nebula-credential, nebula-sdk, nebula-macros.
- `Rule::field_references()` from nebula-validator used for lint cross-referencing.

<!-- reviewed: 2026-03-25 -->
