# Migration

> **Migration authority** for all error‑code and field‑path contract changes.
> Referenced by `error_registry_v1.json` → `change_policy.migration_authority`.

## Versioning Policy

- compatibility promise:
  - minor releases keep behavior-compatible validator semantics.
  - minor releases are additive only for validators/combinators/error helpers.
  - major releases may change semantics with explicit migration steps.
- deprecation window:
  - at least one minor release before removal of deprecated APIs (unless security-critical).

## Error Code Stability

Each error code in `error_registry_v1.json` has a stability level:

| Stability | Meaning |
|-----------|---------|
| `stable` | Will not change meaning or be removed in minor releases. |
| `deprecated` | Scheduled for removal in the next major release. |

### Deprecation Process

1. **Mark** — add `#[deprecated(since = "x.y.z", note = "...")]` to the item.
2. **Registry** — set `"stability": "deprecated"` in `error_registry_v1.json`.
3. **Document** — add an entry to the Breaking Change Mapping table below.
4. **Grace period** — the deprecated item remains for at least one major release cycle.
5. **Remove** — in the next major release, remove the item and record it in the table.

## Field‑Path Contract

All field paths follow **RFC 6901 JSON Pointer** format:

- Root: empty string
- Nested: `/parent/child`
- Array: `/items/0`
- Special chars: `~0` for `~`, `~1` for `/`

Dot‑notation inputs (e.g. `user.email`) are converted to JSON Pointer (`/user/email`)
by `ValidationError::with_field()`. The field‑path format for a given
validator/combinator combination is part of the minor‑release contract.

## Breaking Changes

- validator v2 (in progress):
  - `ValidationError::with_field("a.b[0]")` now canonicalizes into RFC 6901 JSON Pointer (`/a/b/0`).
  - `ValidationError` serialization now includes `pointer` in addition to `field`.
  - new `ValidationError::with_pointer(..)` and `ValidationError::field_pointer()` helpers.
  - consumer adapters should prefer `field_pointer()` over raw `field` access.
  - `nebula-api` switched to `nebula-validator` validation contract in `ValidatedJson` extractor.
  - API validation problems now carry structured field errors (`code`, `pointer`, `detail`).
  - derive macros no longer panic on invalid regex patterns; they emit structured validation errors.
- potential future breaking candidates:
  - typed `FieldPath`
  - explicit fail-fast/collect-all default policy change
  - strict error code registry enforcement

## Rollout Plan

1. preparation:
  - add pointer-native APIs (`with_pointer`, `field_pointer`) and emit pointer in JSON envelope.
2. cutover:
  - normalize all `with_field` inputs to RFC 6901 pointer format.
3. consumer migration:
  - update config/parameter/api adapters to consume pointer paths.
4. cleanup:
  - remove remaining raw-field assumptions in tests and fixtures.

## Rollback Plan

- trigger conditions:
  - consumer breakage in error-code/path contracts.
- rollback steps:
  - revert to previous stable version and restore compatibility mapping.
- data/state reconciliation:
  - ensure persisted validation error envelopes remain parseable by consumers.

## Validation Checklist

- API compatibility checks:
  - compile-time checks for public trait signatures.
- integration checks:
  - consumer fixtures for `api`, `workflow`, `plugin`.
- performance checks:
  - benchmark comparison against previous baseline.

## Breaking Change Mapping Template

Use this table for any behavior-significant change:

| Contract Area | Old Behavior | New Behavior | Version | Consumer Impact | Migration Action |
|---|---|---|---|---|---|
| error code | `<old_code>` | `<new_code>` | `vX.Y.0` | API/UI mapping break | map old->new in adapter |
| field-path format | `<old_path_format>` | `<new_path_format>` | `vX.Y.0` | parser break | update path parser and fixtures |
| combinator semantics | `<old_semantic>` | `<new_semantic>` | `vX.Y.0` | behavior drift | update tests and rollout notes |

Rules:

- major version required for any row that changes semantics.
- mapping section is required before release candidate is cut.

## Config Integration Mapping Requirements

For changes that impact config-validator compatibility:

| Surface | Old Behavior | New Behavior | Impacted Consumer | Required Fixture Update |
|---|---|---|---|---|
| category constants | `<old_category_set>` | `<new_category_set>` | `nebula-config` | `validator_contract_v*.json` |
| validation message semantics | `<old_message_shape>` | `<new_message_shape>` | operator tooling | validator diagnostics contract tests |
| field-path meaning | `<old_field_path_rule>` | `<new_field_path_rule>` | config and API adapters | category/path compatibility tests |
