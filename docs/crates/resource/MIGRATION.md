# Migration

## Versioning Policy

- compatibility promise:
  - maintain trait and manager acquire contracts within major version.
  - add new hooks/events/features in additive manner where possible.
- deprecation window:
  - minimum one minor release before removing deprecated non-critical APIs.

## Breaking Changes

- change:
  - possible typed key introduction as preferred registration interface.
  - old behavior:
    - string IDs are the only canonical registry key.
  - new behavior:
    - typed key API may become primary while keeping string compatibility layer.
  - migration steps:
    1. adopt dual registration APIs.
    2. add typed wrappers in action/runtime code.
    3. remove string-only assumptions after deprecation window.

### Typed key migration path (consumer-focused)

1. define typed key wrappers in consumer crate:
   - `struct DbMain;`
   - map to canonical `ResourceKey` in one place
2. replace string literals in action/runtime call sites:
   - from `acquire("db.main", ctx)`
   - to wrapper-based key conversion + `acquire(&key, ctx)`
3. gate old string helpers behind compatibility module:
   - keep temporary adapters while call sites migrate
4. enforce lint/review rule:
   - no new raw string resource IDs in business logic
5. remove compatibility layer after one minor window
- change:
  - potential refined reload semantics (in-place vs destructive classes).
  - old behavior:
    - full pool swap on every reload.
  - new behavior:
    - selective in-place updates for safe fields.
  - migration steps:
    1. classify config fields and add explicit reload mode.
    2. validate behavior in staging with shadow metrics.

## Rollout Plan

1. preparation
   - publish contract changes and add compatibility shims.
2. dual-run / feature-flag stage
   - run old/new behavior in parallel or shadow mode.
3. cutover
   - switch defaults in major release or explicit opt-in.
4. cleanup
   - remove deprecated shims after migration window.

## Rollback Plan

- trigger conditions:
  - increased fatal acquire errors, cross-crate contract test failures, or latency regressions.
- rollback steps:
  - revert to previous compatible crate version and disable new behavior flags.
- data/state reconciliation:
  - drain and recreate affected pools to ensure clean runtime state.

## Validation Checklist

- API compatibility checks:
  - compile downstream crates (`action`, `runtime`, adapters) against new APIs.
- integration checks:
  - acquire/release/shutdown/reload contract tests.
- performance checks:
  - compare benchmark and stress results to pre-migration baseline.
