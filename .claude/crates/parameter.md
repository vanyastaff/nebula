# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- `ValidatedValues` only constructible via `ParameterCollection::validate()` or `ValidationReport::into_validated()` — `pub(crate)` constructor.
- `ValidationReport` not `Default`; only `ValidationReport::new()` (pub(crate)). `into_validated()` takes no args.
- Loaders are fallible — `Result<LoaderResult<T>, LoaderError>`.
- `Condition` (visibility/required predicate) is separate from `Rule` (value constraint). Never mix.
- `ParameterValues::from_map` and `from_single` are the canonical constructors for nested validation — do not re-inline `.collect()` in validate.rs.

## Key Decisions
- v3 API: `Parameter` struct + `ParameterType` enum (19 variants).
- `ModeVariant` removed — Mode variants are `Vec<Parameter>` (param.id = variant key).
- `DisplayMode` PickFields/Sections skip backfill for absent keys.
- `Transformer` applied lazily via `get_transformed()` — does NOT affect validation/normalization.
- `FieldSpec` restricted subset (4 variants) for dynamic providers.

## Traps
- `docs/crates/parameter/` describes removed v1 APIs — do not use.
- `Rule` re-exported from `nebula_validator` — same type, one source.
- Type-specific builders `debug_assert!` on wrong `ParameterType` variant (panics in debug).
- Unknown fields inside nested objects produce **warnings**, not errors (even in Strict profile).
- `ParameterError::ValidationError` removed — use `ValidationIssue`.
- `input_type` field/method deprecated (0.4.0) — use `.input_hint(InputHint::...)`. Macro still emits `.input_type()` — Task 5 fixes that.

## Relations
- Used by nebula-action (re-exports `Parameter`, `ParameterCollection`), nebula-credential, nebula-sdk, nebula-macros.

<!-- reviewed: 2026-04-07 — from_json_owned added to ParameterValue; takes ownership to avoid clone on literal path; expression/mode detection logic mirrors from_json -->
