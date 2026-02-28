# Migration

## Versioning Policy

- compatibility promise:
  - stable high-level API and core expression semantics within major versions.
- deprecation window:
  - at least one minor release for non-critical removals/renames where possible.

## Breaking Changes

- change:
  - function semantics or naming normalization.
  - old behavior:
    - legacy function behavior/profile.
  - new behavior:
    - normalized or strict-mode function semantics.
  - migration steps:
    1. enable compatibility profile.
    2. update expressions incrementally with lint/report tooling.
    3. switch to new defaults in major release.
- change:
  - grammar/operator precedence updates.
  - old behavior:
    - legacy parse precedence.
  - new behavior:
    - revised precedence/grammar rules.
  - migration steps:
    1. run static expression audit.
    2. patch ambiguous expressions with explicit parentheses.

## Rollout Plan

1. preparation
   - publish change notes and compatibility flags.
2. dual-run / feature-flag stage
   - compare old/new evaluation outputs in staging.
3. cutover
   - enable new semantics by policy or major release.
4. cleanup
   - remove compatibility shims after deprecation window.

## Rollback Plan

- trigger conditions:
  - significant expression mismatch incidents or latency regressions.
- rollback steps:
  - revert to prior compatibility profile/version.
- data/state reconciliation:
  - re-evaluate impacted workflow parameters/templates if output changed.

## Validation Checklist

- API compatibility checks:
  - downstream compile tests for runtime/action/parameter consumers.
- integration checks:
  - expression parity and error-shape regression tests.
- performance checks:
  - benchmark diff vs previous baseline.
