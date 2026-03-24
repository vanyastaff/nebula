# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- **`src/schema.rs` and `src/providers.rs` are the ground truth.** `docs/crates/parameter/*.md` is the old v1 API — stale, kept for migration history. Never trust those docs.
- Provider registration keys must be lowercase ASCII with only `.`, `_`, `-`. Returns `Err` on bad key — never panics.

## Key Decisions
- V2 `Field` enum has 16 variants covering all UI input types. `Schema` is the container.
- `FieldValues` is the runtime key-value map; `ValidatedValues` wraps it post-validation.
- `Condition` handles field visibility/required logic declaratively (show field only when another field = X).
- `ValidationProfile` selects which rules run (strict vs permissive).

## Traps
- **DO NOT use the old docs** — `docs/crates/parameter/` describes removed APIs.
- `Rule` in this crate (`parameter::Rule`) is distinct from `nebula_validator::Rule`. Both exist; context determines which.
- `OptionLoader` / `RecordLoader` are inline async loaders for dynamic select fields — they require an async runtime; don't call in sync contexts.
- `UnknownFieldPolicy` controls whether extra keys in `FieldValues` are rejected or ignored — defaults to strict.

## Relations
- Used by nebula-action (re-exports `Field`, `Schema`), nebula-sdk, nebula-macros.

<!-- reviewed: 2026-03-24 -->
