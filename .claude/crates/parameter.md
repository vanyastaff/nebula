# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

**STATUS: v2→v3 migration in progress** (see `docs/plans/2026-03-24-parameter-v3-migration.md`). During migration, both v2 and v3 types may coexist temporarily. The v3 HLD is `docs/designs/nebula-parameter-hld-v3.md`.

## Invariants
- **v3 ground truth:** `src/parameter.rs` (Parameter struct), `src/parameter_type.rs` (ParameterType enum), `src/collection.rs` (ParameterCollection). Old `src/schema.rs` and `src/field.rs` are being replaced.
- Provider registration keys must be lowercase ASCII with only `.`, `_`, `-`. Returns `Err` on bad key — never panics.
- **`ValidatedValues` is only constructible via `ParameterCollection::validate()` / `validate_with_profile()`.** Constructor is `pub(crate)` — external crates cannot fake validation proof.
- **Loaders are fallible.** `OptionLoader`, `RecordLoader`, `FilterFieldLoader` return `Result<LoaderResult<T>, LoaderError>`. Callers must handle errors.
- **`Condition` is separate from `Rule`.** `Condition` = predicate on sibling values (visibility, required). `Rule` = validation constraint on own value. They never mix.

## Key Decisions
- v3 replaces `Field` enum (16 variants) with `Parameter` struct + `ParameterType` enum (19 variants). Shared metadata is on the struct; type-specific data in the enum.
- `ParameterValues` (renamed from `FieldValues`) is the runtime key-value map; `ValidatedValues` wraps it post-validation.
- `Condition` is its own enum (Eq, Ne, OneOf, Set, NotSet, IsTrue, Gt, Lt, All, Any, Not) using `ParameterPath` for field references.
- `Transformer` pipeline applied lazily via `get_transformed()` — does NOT affect validation or normalization.
- `DisplayMode` controls Object presentation: Inline, Collapsed, PickFields, Sections. PickFields/Sections skip default backfill for absent keys.
- `ModeVariant` wrapper removed — Mode variants are `Vec<Parameter>` directly (param.id = variant key).
- `Notice` and `Computed` are parameter types, not separate UI elements.
- `ValidationProfile` selects which rules run (strict vs permissive). Profile is threaded through nested Object/Mode validation — consistent at all depths.
- `FieldSpec` is an intentionally restricted subset of `Parameter` (4 variants) for dynamic providers.

## Traps
- **DO NOT use the old docs** — `docs/crates/parameter/` describes removed APIs.
- `Rule` in this crate (`parameter::Rule`) is distinct from `nebula_validator::Rule`. Both exist; context determines which.
- `OptionLoader` / `RecordLoader` / `FilterFieldLoader` are inline async loaders — they require an async runtime; don't call in sync contexts. **They return `Result<LoaderResult<T>>` — handle `LoaderError`.**
- v3 type-specific builders (e.g. `multiline()`, `min()`, `searchable()`) silently no-op when called on the wrong ParameterType variant. `with_option_loader()` / `with_record_loader()` / `with_filter_field_loader()` also no-op on wrong variant.
- `ParameterError::ValidationError` variant was removed — use `ValidationIssue` for all structured validation errors.
- **Task 13 completed:** lib.rs wires all v3 modules. Old `field.rs`, `metadata.rs`, `schema.rs` deleted. Integration tests and examples use old API — pending Tasks 14-15.
- **Task 11 completed:** `lint.rs` fully rewritten with 23 diagnostics: structure (duplicate IDs, empty IDs, duplicate mode variants, invalid default_variant), references (unknown condition fields, depends_on non-existent/self-ref, $root.x dangling), rule consistency (min_length>max_length, min_items>max_items), object/mode warnings (Sections missing group, group on non-Sections, required in PickFields, few sub-params, variant missing label), transformer warnings (string-only on non-string, invalid regex, group 0, single Chain/FirstMatch), notice warnings (required/secret/default/rules on notice, missing description), filter warnings (no fields, duplicate field IDs). Integration test `tests/lint.rs` rewritten for v3 API.
- **Task 9 completed:** `validate.rs` fully rewritten. Validates per-parameter (skip Computed/Notice, visible_when, required/required_when, rules, type-specific). Type-specific: Number (integer/min/max), Select (options/multi/allow_custom), Object (recursive with pick-mode), List (min/max items, recursive), Mode (variant matching under "mode"+"value" keys), Dynamic/Filter skipped. Unknown field check per ValidationProfile.
- **Task 10 completed:** `normalize.rs` fully rewritten. Backfills defaults from `Parameter.default` and `Mode.default_variant`. Skips Computed/Notice/Hidden. Mode: ensures "mode" key, recurses into variant (Object→recurse "value", scalar→backfill "value" default, Hidden→skip). Object: Inline/Collapsed recurse all sub-params; PickFields/Sections only process present keys. List: recurse each item with template. Depth-limited to 16. Extra keys preserved. User values never overwritten.

## Relations
- Used by nebula-action (re-exports `Parameter`, `ParameterCollection`), nebula-sdk, nebula-macros.
- `Rule::field_references()` added to nebula-validator for lint cross-referencing.
- Consumers migrating: action, credential, auth, engine, sdk, macros, resource.

<!-- reviewed: 2026-03-25 (Task 11: rewrite lint engine) -->
