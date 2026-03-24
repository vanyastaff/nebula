# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- **`src/schema.rs` and `src/providers.rs` are the ground truth.** `docs/crates/parameter/*.md` is the old v1 API — stale, kept for migration history. Never trust those docs.
- Provider registration keys must be lowercase ASCII with only `.`, `_`, `-`. Returns `Err` on bad key — never panics.
- **`ValidatedValues` is only constructible via `Schema::validate()` / `Schema::validate_with_profile()`.** Constructor is `pub(crate)` — external crates cannot fake validation proof.
- **Loaders are fallible.** `OptionLoader` and `RecordLoader` return `Result<Vec<T>, LoaderError>`. Callers must handle errors.

## Key Decisions
- V2 `Field` enum has 16 variants covering all UI input types. `Schema` is the container.
- `FieldValues` is the runtime key-value map; `ValidatedValues` wraps it post-validation.
- `Condition` handles field visibility/required logic declaratively (show field only when another field = X).
- `ValidationProfile` selects which rules run (strict vs permissive). Profile is threaded through nested Object/Mode validation — consistent at all depths.
- `FieldSpec` is an intentionally restricted subset of `Field` (4 variants) for dynamic providers. Duplication is accepted; `TryFrom<&Field>` and `From<FieldSpec>` conversions available.

## Traps
- **DO NOT use the old docs** — `docs/crates/parameter/` describes removed APIs.
- `Rule` in this crate (`parameter::Rule`) is distinct from `nebula_validator::Rule`. Both exist; context determines which.
- `OptionLoader` / `RecordLoader` are inline async loaders for dynamic select fields — they require an async runtime; don't call in sync contexts. **They return `Result` — handle `LoaderError`.**
- `UnknownFieldPolicy` controls whether extra keys in `FieldValues` are rejected or ignored — defaults to strict.
- `with_option_loader()` / `with_record_loader()` use `debug_assert!` on wrong variant — no-op in release, panics in debug.
- `ParameterError::ValidationError` variant was removed — use `ValidationIssue` for all structured validation errors.

## Relations
- Used by nebula-action (re-exports `Field`, `Schema`), nebula-sdk, nebula-macros.
- `Rule::field_references()` added to nebula-validator for lint cross-referencing.

<!-- reviewed: 2026-03-24 -->
