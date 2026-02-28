# Migration

## Versioning Policy

- compatibility promise:
  - stable tenant context and policy decision contracts within major versions.
- deprecation window:
  - minimum one minor release for non-critical removals.

## Breaking Changes

- change:
  - tenant policy ownership moved from distributed ad-hoc logic to `nebula-tenant` contracts.
  - old behavior:
    - tenant rules implemented inconsistently across crates.
  - new behavior:
    - centralized tenant decision and context API.
  - migration steps:
    1. introduce adapter wrappers in existing crates.
    2. dual-run old+new checks in shadow mode.
    3. cut over to tenant-owned enforcement.
- change:
  - partition strategy model may evolve.
  - old behavior:
    - implicit/legacy strategy assumptions.
  - new behavior:
    - explicit strategy contract and migration tooling.
  - migration steps:
    1. map current tenants to target strategies.
    2. perform staged data/resource migration.

## Rollout Plan

1. preparation
   - implement tenant crate MVP and adapters.
2. dual-run / feature-flag stage
   - shadow policy decisions and compare outcomes.
3. cutover
   - switch enforcement to tenant-owned contracts.
4. cleanup
   - remove legacy duplicated logic.

## Rollback Plan

- trigger conditions:
  - false denials, policy regressions, severe latency impact.
- rollback steps:
  - disable new tenant enforcement flags and revert to previous path.
- data/state reconciliation:
  - reconcile quota counters and policy versions after rollback.

## Validation Checklist

- API compatibility checks:
  - compile and run contract tests for runtime/api/resource/storage/credential.
- integration checks:
  - identity, isolation, quota, and partition scenarios.
- performance checks:
  - admission latency and contention benchmarks before/after.
