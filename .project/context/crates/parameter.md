# nebula-parameter
Parameter schema system (RFC 0005) — defines what inputs a workflow node accepts.

## Invariants
- `ValidatedValues` only via `into_validated()` — `pub(crate)` constructor; `ValidationReport` not `Default`.
- `ValidationReport.errors`/`.warnings` are `pub(crate)`; accessors: `.errors()`/`.warnings()`, `push_error()`/`push_warning()`.
- `Condition` (visibility predicate) is separate from `Rule` (value constraint). Never mix.
- `ParameterValues::from_map`/`from_single` are canonical constructors — don't re-inline in validate.rs.

## Key Decisions
- v3 API: `Parameter` + `ParameterType` (19 variants). `ModeVariant` removed.
- `Transformer` applied lazily via `get_transformed()` — does NOT affect validation.

## Traps
- `docs/crates/parameter/` describes removed v1 APIs — do not use.
- `Rule` re-exported from `nebula_validator` — one source.
- Type-specific builders `debug_assert!` on wrong variant (panics in debug).
- Unknown nested fields → warnings, not errors (even Strict profile).
- `ParameterError::ValidationError` removed — use `ValidationIssue`.
- `input_type` deprecated — macro emits `.input_hint(InputHint::...)`.
- `#[validate(...)]` derive: flat keys `required`, `url`, `email`, `min_length`, `max_length`, `min`, `max`, `pattern` → `.with_rule(Rule::...)`. `min`/`max` are `u64`.
- `#[param(visible_when_field = "f", visible_when_value = "v")]` / `required_when_field/value` — flat key pairs generate `Condition::eq(f, v)` setters. Both keys required; omitting one silently skips the setter.
- `#[derive(Parameters)]` generates `Default` impl only when at least one field has `#[param(default = ...)]`. Combine with `#[serde(default)]` on the struct. Skipped fields use `Default::default()`.

## Relations
- Used by nebula-action, nebula-credential, nebula-sdk.
