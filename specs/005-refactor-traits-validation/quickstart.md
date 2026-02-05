# Quickstart: Remove Duplicates and Rename to Rust Stdlib Style

**Feature**: Remove Duplicate Trait Definitions + Rename Types  
**Date**: 2026-02-04  
**Status**: Implementation Guide

## Summary

This refactoring removes duplicate trait definitions from `rotation/validation.rs` and renames validation types to follow Rust stdlib naming conventions (like `std::io::Result`, `std::io::ErrorKind`).

**What's Changing**:
- ✅ Remove duplicate `TestableCredential` from rotation/validation.rs (~42 lines)
- ✅ Remove duplicate `RotatableCredential` from rotation/validation.rs (~30 lines)
- ✅ Remove unnecessary `TokenRefreshValidator` trait (~76 lines)
- ✅ Add import: `use crate::traits::{TestableCredential, RotatableCredential};`
- ✅ Rename: `ValidationContext` → `TestContext`
- ✅ Rename: `ValidationOutcome` → `TestResult`
- ✅ Rename: `ValidationFailureHandler` → `FailureHandler`
- ✅ Rename: `ValidationFailureType` → `FailureKind`

**What's NOT Changing**:
- ❌ No trait method signatures modified
- ❌ No import paths changed
- ❌ No public API changes
- ❌ No breaking changes
- ❌ No files moved or deleted

**Impact**: ~200 lines removed, better Rust-idiomatic naming, improved maintenance

---

## For Developers: No Action Required

If you're implementing credentials or using the validation framework, **no changes are needed**:

```rust
// ✅ This still works exactly the same
use nebula_credential::prelude::*;

struct MyCredential { /* ... */ }

#[async_trait]
impl Credential for MyCredential {
    // ... implementation unchanged
}

#[async_trait]
impl TestableCredential for MyCredential {
    async fn test(&self) -> RotationResult<TestResult> {
        // ... implementation unchanged (return type updated)
    }
}

#[async_trait]
impl RotatableCredential for MyCredential {
    async fn rotate(&self) -> RotationResult<Self> {
        // ... implementation unchanged
    }
}
```

**Import paths remain identical**:
```rust
// All of these still work:
use nebula_credential::prelude::*;
use nebula_credential::traits::TestableCredential;
use nebula_credential::traits::RotatableCredential;
use nebula_credential::rotation::ValidationContext;
```

---

## What Was Removed

### 1. Duplicate TestableCredential Trait

**Location**: rotation/validation.rs lines 37-79

**Why removed**: Already defined in `traits/testable.rs` (source of truth)

**Before**:
```rust
// rotation/validation.rs had its own copy:
#[async_trait]
pub trait TestableCredential: Send + Sync {
    async fn test(&self) -> RotationResult<ValidationOutcome>;
    fn test_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
```

**After**:
```rust
// rotation/validation.rs now imports from traits/:
use crate::traits::TestableCredential;
```

### 2. Duplicate RotatableCredential Trait

**Location**: rotation/validation.rs lines 81-111

**Why removed**: Already defined in `traits/rotation.rs` (source of truth)

**Before**:
```rust
// rotation/validation.rs had its own copy:
#[async_trait]
pub trait RotatableCredential: TestableCredential {
    async fn rotate(&self) -> RotationResult<Self>
    where
        Self: Sized;

    async fn cleanup_old(&self) -> RotationResult<()> {
        Ok(())
    }
}
```

**After**:
```rust
// rotation/validation.rs now imports from traits/:
use crate::traits::RotatableCredential;
```

### 3. TokenRefreshValidator Trait

**Location**: rotation/validation.rs lines 113-189

**Why removed**: Functionality already covered by `Credential::refresh()` + `CredentialMetadata`

**Before**:
```rust
#[async_trait]
pub trait TokenRefreshValidator: TestableCredential {
    async fn refresh_token(&self) -> RotationResult<Self>
    where
        Self: Sized;

    fn get_expiration(&self) -> Option<chrono::DateTime<chrono::Utc>>;
    fn time_until_expiry(&self) -> Option<chrono::Duration>;
    fn should_refresh(&self, threshold_percentage: f32) -> bool;
}
```

**Why unnecessary**:
- `Credential::refresh()` method already provides token refresh
- `CredentialMetadata::expires_at` tracks expiration time
- `CredentialMetadata::ttl_seconds` tracks time-to-live
- No code in the crate currently uses this trait
- Token refresh is a credential capability, not a validation concern

**Migration** (if you were using it):
```rust
// ❌ OLD (removed):
impl TokenRefreshValidator for OAuth2Credential {
    async fn refresh_token(&self) -> RotationResult<Self> {
        // ...
    }
    fn get_expiration(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

// ✅ NEW (use Credential::refresh instead):
impl Credential for OAuth2Credential {
    async fn refresh(
        &self, 
        state: &mut Self::State,
        ctx: &mut CredentialContext
    ) -> Result<(), CredentialError> {
        // Refresh logic here
        // Update state.expires_at from metadata
    }
}
```

---

## What Stayed the Same

### Testing Types Renamed to Rust Stdlib Style

These types remain in `rotation/validation.rs` but are **renamed** to follow Rust conventions:

| Old Name | New Name | Inspiration |
|----------|----------|-------------|
| `ValidationContext` | `TestContext` | Short, clear context object |
| `ValidationOutcome` | `TestResult` | Like `std::io::Result` |
| `ValidationFailureHandler` | `FailureHandler` | Concise, context-obvious |
| `ValidationFailureType` | `FailureKind` | Like `std::io::ErrorKind` |

**Unchanged types**:
- ✅ `TestMethod` - Test method definitions
- ✅ `SuccessCriteria` - Success criteria definitions
- ✅ `ValidationTest` - Validation test definitions

**Why renamed**: Follow Rust stdlib naming patterns (`Result`, `ErrorKind`) for better idiomaticity.

---

## Verification Steps

After the refactoring is complete, verify:

### 1. Compilation Check
```bash
cargo check --workspace
```
**Expected**: ✅ Success with no errors

### 2. Linting Check
```bash
cargo clippy --workspace -- -D warnings
```
**Expected**: ✅ No warnings or errors

### 3. Test Suite
```bash
cargo test --workspace
```
**Expected**: ✅ All tests pass

### 4. Documentation Build
```bash
cargo doc --no-deps --workspace
```
**Expected**: ✅ Documentation builds without errors

### 5. Search for TokenRefreshValidator
```bash
rg "TokenRefreshValidator" --type rust
```
**Expected**: ✅ No matches (trait completely removed)

### 6. Verify Imports Work
```rust
// Create a test file to verify imports still work:
use nebula_credential::prelude::*;
use nebula_credential::traits::{TestableCredential, RotatableCredential};
use nebula_credential::rotation::{ValidationContext, ValidationOutcome};

// ✅ All imports should compile without errors
```

---

## File Changes Summary

| File | Changes | Lines |
|------|---------|-------|
| `rotation/validation.rs` | Add imports, remove duplicates | -200 |
| `traits/testable.rs` | None | 0 |
| `traits/rotation.rs` | None | 0 |
| `traits/credential.rs` | None | 0 |
| **Total** | | **-200** |

**Files modified**: 1  
**Files deleted**: 0  
**Files created**: 0  

---

## Troubleshooting

### Issue: "TestableCredential not found" error

**Cause**: Missing import in rotation/validation.rs

**Fix**: Add at top of file:
```rust
use crate::traits::{TestableCredential, RotatableCredential};
```

### Issue: "TokenRefreshValidator not found" error

**Cause**: Code still references removed trait

**Fix**: Migrate to `Credential::refresh()`:
```rust
// Replace TokenRefreshValidator usage with:
impl Credential for YourCredential {
    async fn refresh(&self, state: &mut Self::State, ctx: &mut CredentialContext) 
        -> Result<(), CredentialError> 
    {
        // Token refresh logic here
        // Use state.metadata.expires_at for expiration tracking
    }
}
```

### Issue: Tests fail after changes

**Cause**: Tests may have been testing duplicate trait implementations

**Fix**: 
1. Check test file for references to removed traits
2. Update to import from `traits/` instead
3. Remove any tests specific to `TokenRefreshValidator`

---

## Why This Refactoring?

### Problem: Duplicate Trait Definitions

**Before**, the same trait existed in two places:
```
traits/testable.rs        rotation/validation.rs
┌─────────────────┐      ┌─────────────────┐
│ TestableCredential│  ←→ │TestableCredential│ (DUPLICATE)
└─────────────────┘      └─────────────────┘
```

**Issues**:
- ❌ Violates DRY (Don't Repeat Yourself) principle
- ❌ Changes must be made in two places
- ❌ Risk of definitions drifting over time
- ❌ Confusing for contributors (which is correct?)
- ❌ Maintenance burden

**After**, single source of truth:
```
traits/testable.rs        rotation/validation.rs
┌─────────────────┐      ┌─────────────────┐
│ TestableCredential│ ←─── │use traits::*    │
└─────────────────┘      └─────────────────┘
```

**Benefits**:
- ✅ Single source of truth
- ✅ Follows DRY principle
- ✅ Easier maintenance
- ✅ Clear ownership (traits/ owns trait definitions)
- ✅ ~200 lines of code removed

### Research-Backed Decision

From [research.md](./research.md):

> Analysis of major Rust projects (tokio, serde, diesel, validator) confirms:
> 1. Infrastructure traits should be in dedicated modules (traits/)
> 2. Domain-specific types should be close to usage (rotation/)
> 3. Duplicate definitions are an anti-pattern, not a documentation strategy

**Sources**: Validator, Diesel, async-trait projects all maintain single source of truth for trait definitions.

---

## Timeline

| Phase | Activity | Status |
|-------|----------|--------|
| Phase 0 | Research module organization | ✅ Complete |
| Phase 1 | Design data model | ✅ Complete |
| Phase 1 | Create quickstart guide | ✅ Complete |
| Phase 2 | Generate implementation tasks | 🔄 Next: `/speckit.tasks` |
| Phase 3 | Execute refactoring | ⏳ Pending |
| Phase 4 | Verification & testing | ⏳ Pending |
| Phase 5 | Commit & document | ⏳ Pending |

---

## Related Documentation

- [spec.md](./spec.md) - Feature specification
- [plan.md](./plan.md) - Implementation plan
- [research.md](./research.md) - Research findings on Rust module organization
- [data-model.md](./data-model.md) - Detailed trait hierarchy and types

---

## Questions?

**Q: Will this break my credential implementations?**  
A: Type renames are technically breaking, but these types are internal to rotation module. If you use `TestableCredential::test()`, update return type to `TestResult`.

**Q: Do I need to update my code?**  
A: Only if you directly reference `ValidationContext`, `ValidationOutcome`, `ValidationFailureHandler`, or `ValidationFailureType`. Update to new names: `TestContext`, `TestResult`, `FailureHandler`, `FailureKind`.

**Q: Why not move validation types to core/ too?**  
A: Research shows validation should stay domain-specific (rotation module), not generic (core).

**Q: Why remove TokenRefreshValidator?**  
A: `Credential::refresh()` + `CredentialMetadata` already provide this functionality. YAGNI principle.

**Q: Can I still use `use nebula_credential::prelude::*;`?**  
A: Yes, prelude exports are unchanged.

---

**Quickstart Status**: ✅ Complete - Ready for `/speckit.tasks`
