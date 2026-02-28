# Migration

## Versioning Policy

### Compatibility Promise

- **Public API**: Types and functions in `prelude`, crate root re-exports
- **Stable**: `MemoryError`, `MemoryResult`, `ObjectPool`, `Arena`, `ComputeCache`, `MemoryBudget`
- **Experimental**: Modules behind `async`, `profiling`, `adaptive` features
- **Internal**: `allocator::sealed`, `allocator::bump::*` implementation details

### Semver Adherence

| Change Type | Version Bump |
|-------------|--------------|
| New feature (additive) | Minor |
| Bug fix | Patch |
| Breaking API change | Major |
| New feature flag | Minor |
| Feature flag removal | Major |

### Deprecation Window

- Deprecated items annotated with `#[deprecated(since = "X.Y.Z", note = "...")]`
- Minimum one minor release before removal
- Migration guidance in CHANGELOG and this document

## Breaking Changes

### Planned: P001 Unified Config Schema

- **Old behavior**: Separate `PoolConfig`, `CacheConfig`, `BudgetConfig` constructors
- **New behavior**: Optional `MemoryRuntimeConfig` composing all sub-configs
- **Migration steps**:
  1. Continue using individual configs (still supported)
  2. Optionally adopt unified config for new code
  3. No forced migration; individual configs remain available

### Planned: P004 Async Trait Surface

- **Old behavior**: Ad-hoc async wrappers in `async_support` module
- **New behavior**: Consistent `Async*` trait family
- **Migration steps**:
  1. Identify usages of `async_support::*`
  2. Replace with new trait implementations
  3. Update feature flag from `async` (unchanged)

### Historical: None Yet

- No breaking changes have been released
- Pre-1.0: API may change without deprecation cycle

## Rollout Plan

### Phase 1: Preparation

- [ ] Document current API surface in API.md
- [ ] Identify downstream consumers (`nebula-expression`)
- [ ] Create feature flag for new behavior if applicable

### Phase 2: Dual-Run / Feature-Flag Stage

- [ ] Implement new API alongside old
- [ ] Add deprecation warnings to old API
- [ ] Update tests to cover both paths
- [ ] Notify consumers of upcoming change

### Phase 3: Cutover

- [ ] Default to new behavior
- [ ] Old API remains for compatibility window
- [ ] Update documentation to recommend new API

### Phase 4: Cleanup

- [ ] Remove deprecated API after deprecation window
- [ ] Bump major version
- [ ] Update CHANGELOG with migration notes

## Rollback Plan

### Trigger Conditions

- Downstream build failures after upgrade
- Runtime errors not present in previous version
- Performance regression > 20%

### Rollback Steps

1. Pin dependent crate to previous version in `Cargo.toml`
2. Revert any migration changes in consumer code
3. Report issue to `nebula-memory` maintainers

### Data/State Reconciliation

- No persistent state in `nebula-memory`
- In-memory pools/caches rebuilt on restart
- Statistics reset to zero (expected)

## Validation Checklist

### API Compatibility

- [ ] `cargo check -p nebula-expression` succeeds
- [ ] No new warnings with `--all-features`
- [ ] `cargo doc` generates without errors

### Integration

- [ ] `nebula-expression` tests pass
- [ ] Example code in API.md compiles
- [ ] Feature combinations (`pool` + `cache` + `stats`) work together

### Performance

- [ ] Benchmark suite shows no regression > 10%
- [ ] Memory overhead unchanged (within 5%)
- [ ] Startup time unaffected

## Migration Guides

### Upgrading from 0.x to 1.0 (Future)

*Guide will be written when 1.0 is planned.*

### Feature Flag Changes

| Old Flag | New Flag | Notes |
|----------|----------|-------|
| (none yet) | | |

### Import Path Changes

| Old Path | New Path | Notes |
|----------|----------|-------|
| (none yet) | | |
