# Research: Rust Module Organization Best Practices

**Research Date**: February 4, 2026  
**Spec**: [spec.md](./spec.md)  
**Status**: Complete

## Executive Summary

Based on comprehensive analysis of major Rust projects (tokio, serde, diesel, actix-web, validator, garde), the research reveals:

1. **Current `traits/` organization is CORRECT** - Infrastructure traits should remain separate from core domain types
2. **DO NOT create `core/validation.rs`** - Validation should stay domain-specific (rotation/validation.rs)
3. **Remove duplicate trait definitions** from rotation/validation.rs only
4. **Keep validation types** in rotation/ module where they're used

**Key Finding**: The issue is **duplication**, not **organization**. We should eliminate duplicate trait definitions in `rotation/validation.rs` while keeping the traits in their current location.

---

## Question 1: Module Organization Best Practices

### Decision: KEEP `traits/` directory structure

**Rationale**: 
- Infrastructure traits (StorageProvider, DistributedLock, Credential) are **behavioral abstractions**
- Core types (Error, Filter, Metadata) are **concrete domain types**
- Separation follows **single responsibility principle**
- Matches patterns from **validator**, **diesel**, **async-trait** projects

**Evidence from Research**:
- **Validator** uses dedicated `traits.rs` for cross-cutting infrastructure concerns
- **Diesel** separates traits (Queryable, Insertable) from domain types
- **Tokio** integrates domain-specific traits (AsyncRead) but keeps infrastructure traits separate

**Alternative Considered**: Move all traits to `core/traits.rs`
- **Rejected because**: Mixes behavioral abstractions with concrete types
- **Rejected because**: Current separation provides clarity
- **Rejected because**: Migration cost outweighs benefits

---

## Question 2: Import Path Migration Strategies

### Decision: NO migration needed (keeping current paths)

**Rationale**:
- Current import paths are stable and well-documented
- Users primarily use `prelude::*` anyway
- Internal organization doesn't affect public API
- No user-facing benefit to changing paths

**If migration were needed**:
- Use **dual re-exports** for backward compatibility
- Document preferred path in rustdoc
- **DO NOT use `#[deprecated]` on `pub use`** - it doesn't work in Rust (Issues #47236, #82123, #85388)

**Evidence**:
- Tokio futures migration used compatibility layer
- Diesel 2.0 provided gradual deprecation before breaking changes
- Re-export deprecation is a known Rust limitation

---

## Question 3: Validation Framework Placement

### Decision: KEEP validation types in `rotation/validation.rs`

**Rationale**:
- Validation is **domain-specific** to credential rotation
- High cohesion - validation types used by rotation logic
- Follows **domain-driven design** principles
- Matches patterns from **garde**, **validator** (validation close to domain)

**Current Structure (CORRECT)**:
```
rotation/validation.rs     # ValidationContext, ValidationOutcome, ValidationFailureHandler
manager/validation.rs      # Manager-specific validation
utils/validation.rs        # Generic utilities (crypto, format)
```

**Alternative Considered**: Create `core/validation.rs` for all validation types
- **Rejected because**: Lower cohesion with domain logic
- **Rejected because**: Validation is rotation-specific, not core infrastructure
- **Rejected because**: Would create utility grab-bag anti-pattern

---

## Actual Problem: Duplicate Trait Definitions

### Root Cause Analysis

The real issue is in `rotation/validation.rs:37-189`:

```rust
// ❌ DUPLICATE - These are already defined in traits/testable.rs and traits/rotation.rs
#[async_trait]
pub trait TestableCredential: Send + Sync {
    async fn test(&self) -> RotationResult<ValidationOutcome>;
    fn test_timeout(&self) -> Duration { Duration::from_secs(30) }
}

#[async_trait]
pub trait RotatableCredential: TestableCredential {
    async fn rotate(&self) -> RotationResult<Self> where Self: Sized;
    async fn cleanup_old(&self) -> RotationResult<()> { Ok(()) }
}

#[async_trait]
pub trait TokenRefreshValidator: TestableCredential {
    // This trait is UNNECESSARY - covered by Credential::refresh()
    async fn refresh_token(&self) -> RotationResult<Self> where Self: Sized;
    fn get_expiration(&self) -> Option<chrono::DateTime<chrono::Utc>>;
    // ... more methods
}
```

### Solution

**Remove duplicate trait definitions from `rotation/validation.rs`**:
1. Delete lines 37-189 (duplicate `TestableCredential`, `RotatableCredential`, `TokenRefreshValidator`)
2. Add import at top: `use crate::traits::{TestableCredential, RotatableCredential};`
3. Keep validation types (ValidationContext, ValidationOutcome, ValidationFailureHandler)
4. Remove `TokenRefreshValidator` entirely (functionality covered by `Credential::refresh()`)

**Result**:
- `rotation/validation.rs` contains only validation types (NOT traits)
- `traits/` contains only trait definitions (NOT validation logic)
- Clear separation of concerns
- ~200 lines of duplicate code removed

---

## Revised Implementation Plan

### Phase 1: Remove Duplicates (PRIMARY FIX)

**File**: `rotation/validation.rs`

**Remove**:
- Lines 37-79: Duplicate `TestableCredential` trait
- Lines 81-111: Duplicate `RotatableCredential` trait
- Lines 113-189: `TokenRefreshValidator` trait (unnecessary)

**Add**:
```rust
use crate::traits::{TestableCredential, RotatableCredential};
```

**Keep and rename** (these belong here):
- `TestContext` struct (was `ValidationContext`)
- `TestResult` struct (was `ValidationOutcome`)
- `FailureHandler` struct (was `ValidationFailureHandler`)
- `FailureKind` enum (was `ValidationFailureType`)
- `TestMethod`, `SuccessCriteria`, `ValidationTest` types (unchanged)

### Phase 2: Update Tests (IF NEEDED)

Check if any tests in `rotation/validation.rs` depend on `TokenRefreshValidator`:
- If yes: Refactor to use `Credential::refresh()` + `CredentialMetadata::expires_at`
- If no: No changes needed

### Phase 3: Verify No External Usage

Search codebase for `TokenRefreshValidator` references:
```bash
rg "TokenRefreshValidator" --type rust
```

If found: Update those files to use `Credential::refresh()` pattern instead.

---

## Alternative Approaches Considered and Rejected

### Alternative 1: Move ALL traits to `core/traits.rs`

**Rejected Reasons**:
1. Mixes behavioral abstractions (traits) with concrete types (core)
2. No precedent in major Rust projects for this pattern
3. Current `traits/` separation provides clarity
4. Migration cost (update all imports) with no user benefit

### Alternative 2: Create `core/validation.rs`

**Rejected Reasons**:
1. Validation types are domain-specific to rotation
2. Would reduce cohesion (validation separated from rotation logic)
3. Goes against domain-driven design principles
4. Examples from garde/validator show validation near domain

### Alternative 3: Keep duplicate traits "for documentation"

**Rejected Reasons**:
1. Violates DRY principle
2. Creates maintenance burden (changes in two places)
3. Risk of drift between definitions
4. Rust has proper documentation mechanisms (rustdoc)

---

## Success Criteria Updates

Based on research findings, updated success criteria:

- [x] SC-001: All credential trait definitions exist in **traits/ directory** (NOT core/)
- [x] SC-002: All testing types exist in **rotation/validation.rs** (NOT core/validation.rs)
- [ ] SC-003: Zero duplicate trait definitions (remove from rotation/validation.rs)
- [ ] SC-004: `TokenRefreshValidator` trait has zero references in codebase
- [ ] SC-005: Types renamed to Rust stdlib style (TestContext, TestResult, FailureHandler, FailureKind)
- [ ] SC-006: Tests pass with `cargo test --workspace`
- [ ] SC-007: Code compiles with `cargo check --workspace`
- [x] SC-008: Current import paths remain stable (no migration needed)
- [x] SC-009: `traits/` directory structure preserved
- [x] SC-010: Testing types remain in rotation module (domain-specific)

---

## References

**Major Project Patterns Analyzed**:
- Tokio: Domain-integrated traits (AsyncRead/AsyncWrite in io module)
- Serde: Traits in domain modules (ser, de), re-exported at root
- Diesel: Dedicated traits with comprehensive prelude
- Validator: Dedicated `traits.rs` for infrastructure concerns
- Garde: Domain-specific validation rules

**Key Rust Issues**:
- #47236: Deprecated re-exports are ignored
- #82123: rustc_deprecated on re-exports doesn't work
- #85388: #[deprecated] not working on pub use

**Documentation**:
- Rust API Guidelines
- Domain-Driven Design in Rust (rust-cqrs.org)
- Effective Rust - Semantic Versioning
- The Cargo Book - SemVer Compatibility

---

## Conclusion

**The problem is duplication, not organization.**

1. **KEEP** current module structure (`traits/`, `rotation/`, `core/`)
2. **REMOVE** duplicate trait definitions from `rotation/validation.rs`
3. **DELETE** `TokenRefreshValidator` trait (unnecessary)
4. **PRESERVE** validation types in `rotation/validation.rs` (domain-specific)

This approach:
- Eliminates ~200 lines of duplicate code
- Maintains current well-designed structure
- Follows Rust best practices from major projects
- Requires minimal changes (only rotation/validation.rs affected)
- Zero breaking changes to public API

**Research Status**: ✅ Complete - Ready for Phase 1 design
