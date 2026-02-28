# Migration

## Versioning Policy

- compatibility promise:
  - minor releases keep behavior-compatible validator semantics.
  - minor releases are additive only for validators/combinators/error helpers.
  - major releases may change semantics with explicit migration steps.
- deprecation window:
  - at least one minor release before removal of deprecated APIs (unless security-critical).

## Breaking Changes

- currently planned:
  - none committed.
- potential future breaking candidates:
  - typed `FieldPath`
  - explicit fail-fast/collect-all default policy change
  - strict error code registry enforcement

## Rollout Plan

1. preparation:
  - introduce new APIs in additive form behind clear docs.
2. dual-run / feature-flag stage:
  - allow old and new behavior side-by-side where possible.
3. cutover:
  - switch defaults only in major release.
4. cleanup:
  - remove deprecated path after migration window.

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
