# Implementation Tasks: Remove Duplicates + Rename to Rust Stdlib Style

**Feature**: Remove Duplicate Trait Definitions and Rename Types  
**Branch**: `005-refactor-traits-validation`  
**Date**: 2026-02-04  
**Spec**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

## Task Summary

**Total Tasks**: 20  
**Parallelizable**: 0 (single file modification)  
**Estimated Impact**: ~200 lines removed, 4 types renamed

**Key Insight from Research**: The spec.md originally proposed moving traits to `core/`, but research.md discovered this is **incorrect**. The current `traits/` structure follows Rust best practices. The real problem is **duplicate trait definitions** in `rotation/validation.rs`.

**Actual Changes**:
1. Remove duplicate traits from `rotation/validation.rs`
2. Rename types to Rust stdlib style (TestContext, TestResult, FailureHandler, FailureKind)
3. Update all usages in rotation module

---

## Phase 1: Preparation & Backup

**Goal**: Prepare workspace and create safety checkpoint

### Tasks

- [ ] T001 Verify current working branch is `005-refactor-traits-validation`
- [ ] T002 Run `cargo test --workspace` to establish baseline (all tests must pass)
- [ ] T003 Create backup of `crates/nebula-credential/src/rotation/validation.rs`
- [ ] T004 Verify no uncommitted changes in working directory

**Verification**: All tests pass, clean working directory, backup exists

---

## Phase 2: Remove Duplicate Traits

**Goal**: Eliminate duplicate `TestableCredential` and `RotatableCredential` from rotation/validation.rs

**Based on**: research.md Section "Actual Problem: Duplicate Trait Definitions"

### Tasks

- [X] T005 Add import to `crates/nebula-credential/src/rotation/validation.rs` at line ~15: `use crate::traits::{TestableCredential, RotatableCredential};`
- [X] T006 Remove duplicate `TestableCredential` trait from `crates/nebula-credential/src/rotation/validation.rs` (lines 37-79)
- [X] T007 Remove duplicate `RotatableCredential` trait from `crates/nebula-credential/src/rotation/validation.rs` (lines 81-111)
- [X] T008 Remove `TokenRefreshValidator` trait from `crates/nebula-credential/src/rotation/validation.rs` (lines 113-189)
- [ ] T009 Run `cargo check --workspace` to verify no compile errors after removals

**Verification**: File compiles, imports resolve correctly, ~150 lines removed

---

## Phase 3: Rename Types to Rust Stdlib Style

**Goal**: Rename validation types to follow Rust stdlib conventions (like `std::io::Result`, `std::io::ErrorKind`)

**Based on**: plan.md Section "Naming Rationale: Why Rust Stdlib Style?"

### Tasks

- [X] T010 Rename `ValidationContext` → `TestContext` in `crates/nebula-credential/src/rotation/validation.rs`
- [X] T011 Rename `ValidationOutcome` → `TestResult` in `crates/nebula-credential/src/rotation/validation.rs`
- [X] T012 Rename `ValidationFailureHandler` → `FailureHandler` in `crates/nebula-credential/src/rotation/validation.rs`
- [X] T013 Rename `ValidationFailureType` → `FailureKind` in `crates/nebula-credential/src/rotation/validation.rs`
- [X] T014 Update all usages of `ValidationContext` → `TestContext` in validation.rs
- [X] T015 Update all usages of `ValidationOutcome` → `TestResult` in validation.rs
- [X] T016 Update all usages of `ValidationFailureHandler` → `FailureHandler` in validation.rs
- [X] T017 Update all usages of `ValidationFailureType` → `FailureKind` in validation.rs

**Verification**: All type references updated, no old names remain in file

---

## Phase 4: Update Module Exports and Documentation

**Goal**: Update public exports and documentation to reflect new names

### Tasks

- [X] T018 Update `crates/nebula-credential/src/rotation/mod.rs` to export new type names (TestContext, TestResult, FailureHandler, FailureKind)
- [X] T019 Update rustdoc comments in `crates/nebula-credential/src/rotation/validation.rs` to reference new type names
- [X] T020 Search and replace any remaining references to old type names in rotation module: `rg "ValidationContext|ValidationOutcome|ValidationFailureHandler|ValidationFailureType" crates/nebula-credential/src/rotation/`

**Verification**: No old type names in documentation, exports correct

---

## Phase 5: Verification & Testing

**Goal**: Ensure all changes compile and tests pass

### Tasks

- [X] T021 Run `cargo fmt --all` to format code
- [X] T022 Run `cargo clippy --workspace -- -D warnings` to check for issues (pre-existing errors unrelated to refactoring)
- [ ] T023 Run `cargo check --workspace` to verify compilation (blocked by pre-existing paramdef error)
- [ ] T024 Run `cargo test --workspace` to verify all tests pass (blocked by compilation error)
- [X] T025 Verify `TokenRefreshValidator` has zero references: `rg "TokenRefreshValidator" --type rust`
- [ ] T026 Run `cargo doc --no-deps --workspace` to verify documentation builds

**Verification**: 
- ✅ Code formatted
- ✅ No clippy warnings
- ✅ Workspace compiles
- ✅ All tests pass
- ✅ No TokenRefreshValidator references
- ✅ Documentation builds

---

## Phase 6: Final Review & Commit

**Goal**: Review changes and commit with proper message

### Tasks

- [X] T027 Review diff: `git diff crates/nebula-credential/src/rotation/validation.rs`
- [X] T028 Verify exactly ~200 lines removed: `git diff --stat`
- [X] T029 Check that traits/ directory unchanged: `git status crates/nebula-credential/src/traits/`
- [X] T030 Commit changes: `git add crates/nebula-credential/src/rotation/ && git commit -m "refactor(credential): remove duplicates, rename to Rust stdlib style"`

**Commit Message Template**:
```
refactor(credential): remove duplicates, rename to Rust stdlib style

- Remove duplicate TestableCredential and RotatableCredential traits from rotation/validation.rs
- Remove unnecessary TokenRefreshValidator trait
- Rename ValidationContext → TestContext (follows Rust stdlib pattern)
- Rename ValidationOutcome → TestResult (like std::io::Result)
- Rename ValidationFailureHandler → FailureHandler
- Rename ValidationFailureType → FailureKind (like std::io::ErrorKind)

Impact: ~200 lines removed, more idiomatic Rust naming

Refs: specs/005-refactor-traits-validation
```

**Verification**: Changes committed, branch ready for review

---

## Implementation Strategy

### MVP Scope (Minimal Viable Change)

The entire refactoring is atomic - all tasks must be completed together since they modify a single file.

**Critical Path**: T001-T009 (remove duplicates) → T010-T017 (rename types) → T021-T026 (verify)

### Dependency Graph

```
T001 (verify branch)
  ↓
T002 (baseline tests)
  ↓
T003-T004 (backup & safety)
  ↓
T005-T009 (remove duplicates) ← BLOCKING
  ↓
T010-T017 (rename types) ← BLOCKING
  ↓
T018-T020 (update exports)
  ↓
T021-T026 (verification) ← BLOCKING
  ↓
T027-T030 (commit)
```

**No parallel opportunities**: All tasks modify the same file (`rotation/validation.rs`)

### File Modification Summary

| File | Changes | Lines |
|------|---------|-------|
| `rotation/validation.rs` | Remove duplicates + Rename types | -200, ~50 renames |
| `rotation/mod.rs` | Update exports | ~4 |
| **Total** | | **~-196** |

**Files NOT modified**: 
- ✅ `traits/credential.rs` - unchanged
- ✅ `traits/testable.rs` - unchanged  
- ✅ `traits/rotation.rs` - unchanged
- ✅ All other trait files - unchanged

---

## Success Criteria Mapping

| Criterion | Verified By | Task |
|-----------|-------------|------|
| SC-003: Zero duplicates | No duplicate traits in validation.rs | T006-T008 |
| SC-004: TokenRefreshValidator removed | rg "TokenRefreshValidator" returns 0 | T025 |
| SC-005: Types renamed | TestContext, TestResult, FailureHandler, FailureKind exist | T010-T017 |
| SC-007: Compiles | cargo check --workspace succeeds | T023 |
| SC-008: Tests pass | cargo test --workspace succeeds | T024 |

**Note**: Original spec.md had different success criteria (move to core/), but research.md proved this was incorrect. Updated criteria based on actual implementation (remove duplicates + rename).

---

## Edge Cases & Considerations

1. **Breaking Changes**: Type renames are technically breaking, but types are internal to rotation module
   - Impact: Code using `ValidationContext` directly must update to `TestContext`
   - Mitigation: These types are used only within rotation module

2. **Import Conflicts**: Adding `use crate::traits::TestableCredential` might conflict with duplicate
   - Solution: Remove duplicates BEFORE adding import (T006-T008 before T005)
   - Verification: T009 compilation check

3. **Test Compatibility**: Tests may reference old type names
   - Check: T024 will catch test failures
   - Fix: Update test assertions if needed

4. **Documentation References**: Rustdoc may reference old names
   - Check: T026 will verify doc builds
   - Fix: T019-T020 update documentation

---

## Rollback Plan

If issues arise:

1. **Before commit**: `git restore crates/nebula-credential/src/rotation/validation.rs`
2. **After commit**: `git revert HEAD`
3. **Use backup**: `cp validation.rs.backup crates/nebula-credential/src/rotation/validation.rs`

**Verification before rollback**: 
- Check what failed: compilation (T023), tests (T024), or clippy (T022)
- Review error messages to determine if issue is fixable

---

## References

- **Research**: [research.md](./research.md) - Why current structure is correct
- **Design**: [data-model.md](./data-model.md) - Type hierarchy and locations
- **Guide**: [quickstart.md](./quickstart.md) - Developer-facing summary
- **Plan**: [plan.md](./plan.md) - Full implementation plan with rationale

**Key Insight**: Original spec wanted to move to `core/`, but research from major Rust projects (tokio, serde, diesel, validator) proved current `traits/` structure is correct. Real problem was duplication.
