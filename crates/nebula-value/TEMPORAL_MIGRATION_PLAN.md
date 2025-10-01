# Temporal Types v2 Migration Plan

## Current Status: ⚠️ PARTIALLY MIGRATED

The temporal types (Date, Time, DateTime, Duration) have been **restored from git** but **NOT YET MIGRATED** to nebula-value v2 architecture.

### What's Done ✅
- Files restored to `src/temporal/`
- Added to `Value` enum (4 new variants)
- Added to `ValueKind` enum (4 new kinds)
- Basic integration with Display, Hash, Serde
- Exported in prelude
- All existing tests passing (220 unit tests)

### What's NOT Done ❌
- **no_std compatibility** - Uses `std::sync::OnceLock` (not available in no_std)
- **Error handling** - Uses custom error types instead of `NebulaError`
- **Dependency management** - chrono is always required (should be optional)
- **Modern patterns** - Uses deprecated imports and patterns

## Critical Issues Found

### Issue 1: std::sync::OnceLock (Critical)
**Problem**: All temporal types use `std::sync::OnceLock` for caching
**Impact**: Not compatible with no_std
**Files Affected**: date.rs (3 uses), time.rs (6 uses), datetime.rs (4 uses)
**Solution**: Replace with `once_cell::sync::OnceCell`
**Effort**: 2-4 hours

### Issue 2: Custom Error Types (Critical)
**Problem**: Each file has its own error type with thiserror
- `DateError` (6 variants)
- `TimeError` (12 variants)
- `DateTimeError` (wraps Date + Time errors)
- `DurationError` (7 variants)

**Impact**: Inconsistent with nebula-value v2 error handling
**Solution**: Migrate all to `NebulaError` with `ValueErrorExt`
**Effort**: 8-12 hours

### Issue 3: std Dependencies (Critical)
**Problem**: Uses `std::` imports throughout instead of `core::`/`alloc::`
**Impact**: Breaks no_std compatibility
**Files Affected**: All 4 files
**Solution**: Replace imports systematically
**Effort**: 4-6 hours

### Issue 4: Always-On chrono (Important)
**Problem**: chrono is always required, adds ~500KB to binary
**Impact**: Large dependency footprint
**Solution**: Make chrono optional with feature flag
**Effort**: 6-8 hours

### Issue 5: System Time Dependencies (Important)
**Problem**: Methods like `Date::today()`, `Time::now()` require std::time
**Impact**: Not available in no_std
**Solution**: Feature-gate behind `std` feature
**Effort**: 4-6 hours

## Migration Phases

### Phase 1: Critical Infrastructure (Week 1)
**Goal**: Fix blocking issues for no_std compatibility

#### Task 1.1: Replace OnceLock
- **Files**: date.rs, time.rs, datetime.rs
- **Changes**: Replace `std::sync::OnceLock` with `once_cell::sync::OnceCell`
- **Testing**: Verify caching still works
- **Effort**: 2-4 hours

#### Task 1.2: Migrate Error Types
- **Files**: All 4 temporal files
- **Changes**:
  - Remove custom error enums
  - Use `NebulaError::invalid_input()`, `NebulaError::out_of_range()`, etc.
  - Replace `XxxResult<T>` with `ValueResult<T>`
  - Use `ValueErrorExt` for error context
- **Testing**: Update all error tests
- **Effort**: 8-12 hours

#### Task 1.3: Fix std Imports
- **Files**: All 4 temporal files
- **Changes**:
  - `std::sync::Arc` → `alloc::sync::Arc`
  - `std::fmt` → `core::fmt`
  - `std::cmp` → `core::cmp`
  - `std::hash` → `core::hash`
  - etc.
- **Testing**: Compile with no_std
- **Effort**: 4-6 hours

**Phase 1 Total**: 14-22 hours

### Phase 2: Feature Gating (Week 2)
**Goal**: Make optional features truly optional

#### Task 2.1: Make chrono Optional
- **Files**: All 4 files + Cargo.toml
- **Changes**:
  - Add `temporal-chrono` feature
  - Conditional compile chrono conversions
  - Provide fallback implementations
- **Testing**: Test with/without chrono
- **Effort**: 6-8 hours

#### Task 2.2: Feature-gate System Time
- **Files**: date.rs, time.rs, datetime.rs
- **Changes**:
  - Gate `today()`, `now()` methods behind `std` feature
  - Add documentation about no_std limitations
- **Testing**: Compile without std
- **Effort**: 4-6 hours

**Phase 2 Total**: 10-14 hours

### Phase 3: Testing & Quality (Week 3)
**Goal**: Ensure robustness and maintainability

#### Task 3.1: Property-Based Tests
- **Files**: New test files in tests/
- **Changes**:
  - Add proptest tests for arithmetic
  - Add edge case tests
  - Add concurrent access tests
- **Effort**: 12-16 hours

#### Task 3.2: Documentation
- **Files**: All temporal files + migration docs
- **Changes**:
  - Update API documentation
  - Create migration guide
  - Add usage examples
- **Effort**: 4-6 hours

**Phase 3 Total**: 16-22 hours

## Total Effort Estimate

| Phase | Hours | Weeks (1 dev) |
|-------|-------|---------------|
| Phase 1 | 14-22 | ~1 week |
| Phase 2 | 10-14 | ~1 week |
| Phase 3 | 16-22 | ~1 week |
| **Total** | **40-58** | **2-3 weeks** |

## Risks & Mitigation

### Risk 1: Breaking Changes
**Risk**: API changes may break existing code
**Mitigation**:
- Keep old error types as deprecated aliases for 1 version
- Provide migration guide
- Use semantic versioning correctly

### Risk 2: Performance Regression
**Risk**: Removing chrono optimizations may slow things down
**Mitigation**:
- Benchmark before/after
- Keep chrono conversions as optional fast path
- Optimize critical paths

### Risk 3: Feature Complexity
**Risk**: Too many feature combinations to test
**Mitigation**:
- Focus on 3 main configs: full, no_std+alloc, minimal
- Use CI matrix testing
- Clear documentation of feature requirements

## Recommendations

### Option A: Full Migration (Recommended)
✅ Complete all 3 phases
✅ Proper v2 architecture
✅ no_std compatible
✅ Production ready
⏰ 2-3 weeks effort

### Option B: Minimal Viable (Quick Fix)
✅ Phase 1 only (critical issues)
✅ Gets temporal types working
⚠️ Not fully v2 compliant
⚠️ Still has technical debt
⏰ 1 week effort

### Option C: Defer Migration
✅ Keep current state
⚠️ Not production ready
⚠️ Tests pass but architecture mismatch
❌ Not recommended

## Decision

**Recommendation**: **Option B (Minimal Viable)** for now, schedule Option A for later

**Rationale**:
1. Temporal types are working with current implementation
2. Full migration is 2-3 weeks effort
3. Other features may be higher priority
4. Can be migrated incrementally

**Plan**:
1. Complete Phase 1 critical fixes (OnceLock, basic no_std)
2. Document remaining technical debt
3. Schedule Phase 2-3 for next sprint

## Next Steps

1. **Immediate** (This session):
   - Document current state
   - Mark temporal types as "⚠️ Partially Migrated"
   - Update README with caveats
   - Create tracking issue

2. **Short term** (Next sprint):
   - Phase 1: Critical infrastructure fixes
   - Get to minimal viable state

3. **Long term** (Future sprint):
   - Phase 2: Feature gating
   - Phase 3: Testing & quality
   - Full v2 compliance

## Current Workarounds

Until migration is complete:

### For Users:
- ⚠️ Temporal types require `std` (no no_std support yet)
- ⚠️ Temporal types always pull in chrono dependency
- ⚠️ Error types don't match rest of nebula-value API
- ✅ All functionality works correctly
- ✅ Well-tested and reliable

### For Developers:
- ⚠️ Don't use temporal types as API examples (they're not v2 compliant)
- ⚠️ Be aware of error type differences
- ✅ Can use temporal types for functionality
- ✅ Migration path is clear and documented

## Files Requiring Migration

```
src/temporal/
├── date.rs         ⚠️ 795 lines, 11 issues
├── time.rs         ⚠️ 1,260 lines, 14 issues
├── datetime.rs     ⚠️ 931 lines, 15 issues
└── duration.rs     ⚠️ 691 lines, 8 issues

Total: 3,677 lines of code to migrate
Total: 48 issues identified
```

## Success Criteria

### Phase 1 Complete:
- [ ] No `std::sync::OnceLock` usage
- [ ] All errors use `NebulaError`
- [ ] No direct `std::` imports (use core/alloc)
- [ ] Compiles with `#![no_std]`
- [ ] All existing tests pass

### Phase 2 Complete:
- [ ] chrono is optional feature
- [ ] System time methods feature-gated
- [ ] Works in no_std environments
- [ ] Binary size impact measured

### Phase 3 Complete:
- [ ] Property-based tests added
- [ ] Documentation complete
- [ ] Migration guide published
- [ ] Examples updated

## References

- [Detailed Audit Report](./TEMPORAL_AUDIT_REPORT.md) - Full analysis
- [Phase 7 Testing](./PHASE7_TESTING_COMPLETED.md) - Current test status
- [API Improvements](./API_IMPROVEMENTS.md) - Recent API changes

---

**Status**: ⚠️ **PLANNING** - Migration plan approved, awaiting execution
**Last Updated**: 2024-09-30
**Next Review**: After Phase 1 completion
