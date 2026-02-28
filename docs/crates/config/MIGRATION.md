# Migration

## Versioning Policy

- compatibility promise:
  - additive source/validator/watcher APIs in minor releases.
  - precedence/path semantic changes only in major releases.
- deprecation window:
  - keep deprecated APIs for at least one minor cycle unless security-critical.

## Breaking Changes

- likely future candidates:
  - merge strategy profile defaults
  - typed path API replacing free-form string paths
  - remote source trust policy enforcement defaults

## Rollout Plan

1. preparation:
  - introduce new behavior behind explicit builder options.
2. dual-run / feature-flag stage:
  - run old and new semantics in validation mode to compare outcomes.
3. cutover:
  - switch defaults in major release.
4. cleanup:
  - remove deprecated behavior paths after migration window.

## Rollback Plan

- trigger conditions:
  - unexpected consumer breakage in precedence/path/typed retrieval behavior.
- rollback steps:
  - revert to previous compatible release and source settings.
- data/state reconciliation:
  - ensure last-known-good active config snapshot remains valid and restorable.

## Validation Checklist

- API compatibility checks:
  - compile and integration checks against consumer crates.
- integration checks:
  - startup and reload scenarios with mixed source sets.
- performance checks:
  - compare load/reload/read latency against baseline.

## Mapping Template (Required For Breaking Contract Changes)

Use this template for precedence/path/validation semantic changes:

| Contract Surface | Old Behavior | New Behavior | Consumer Impact | Mitigation | Removal Release |
|---|---|---|---|---|---|
| precedence | `defaults < file < env < inline` | ... | ... | ... | ... |
| path access | missing key -> `PathError` | ... | ... | ... | ... |
| typed retrieval | decode failure -> `TypeError` | ... | ... | ... | ... |
| validation gate | invalid candidate rejected | ... | ... | ... | ... |

Minimum requirements:
- include explicit old -> new mapping for every behavior-significant change.
- provide rollout and rollback steps for consumer crates.

## Config-Validator Contract Mapping (Required)

For changes that affect config-validator trait bridge behavior or category naming:

| Surface | Old Behavior | New Behavior | Consumer Impact | Required Action |
|---|---|---|---|---|
| validator gate | `<old_activation_rule>` | `<new_activation_rule>` | startup/reload behavior shift | update contract tests + runbook |
| category names | `<old_category>` | `<new_category>` | error envelope parser drift | add old->new mapping fixture |
| diagnostics context | `<old_context_format>` | `<new_context_format>` | operator tooling changes | update incident docs and consumers |
