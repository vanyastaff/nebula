# Feature Specification: Refactor Traits and Validation to Core Module

**Feature Branch**: `005-refactor-traits-validation`  
**Created**: 2026-02-04  
**Status**: Draft  
**Input**: User description: "Refactor credential traits and validation to core module - consolidate traits/ into core/traits.rs and extract validation types into core/validation.rs"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Developer imports credential traits from core module (Priority: P1)

A developer working on credential implementations needs to import the core traits (`Credential`, `TestableCredential`, `RotatableCredential`) to implement a new credential type (e.g., PostgreSQL database credentials).

**Why this priority**: This is the foundational change - all trait definitions must be in a single, predictable location before any other work can proceed.

**Independent Test**: Developer can successfully implement a new credential type by importing only from `nebula_credential::core::traits` without any reference to deprecated `traits/` module.

**Acceptance Scenarios**:

1. **Given** a developer needs to implement a new credential type, **When** they import traits, **Then** all required traits (`Credential`, `TestableCredential`, `RotatableCredential`, `InteractiveCredential`) are available from `use nebula_credential::core::traits::*`
2. **Given** existing credential implementations, **When** imports are updated to `core::traits`, **Then** all implementations compile without errors
3. **Given** the old `traits/` directory, **When** it is removed, **Then** no import errors occur in the codebase

---

### User Story 2 - Developer uses validation framework for credential testing (Priority: P2)

A developer implementing credential rotation needs to use the validation framework (`ValidationContext`, `ValidationOutcome`, `ValidationFailureHandler`) to test new credentials before committing rotation.

**Why this priority**: Validation is essential for safe credential rotation, but can work independently from trait consolidation.

**Independent Test**: Developer can validate credentials and handle failures using types from `nebula_credential::core::validation` module.

**Acceptance Scenarios**:

1. **Given** a new credential needs testing, **When** developer creates `ValidationContext`, **Then** they can test the credential with timeout enforcement
2. **Given** a validation failure, **When** developer uses `ValidationFailureHandler`, **Then** transient vs permanent failures are correctly classified
3. **Given** validation types in `core/validation.rs`, **When** rotation module imports them, **Then** no duplicate definitions exist

---

### User Story 3 - Developer implements token refresh without TokenRefreshValidator (Priority: P3)

A developer implementing OAuth2 credentials needs to refresh tokens using the existing `Credential::refresh()` method instead of a separate `TokenRefreshValidator` trait.

**Why this priority**: This simplifies the API by removing an unnecessary trait, but existing functionality already works.

**Independent Test**: OAuth2 and JWT credentials can refresh tokens using only `Credential::refresh()` method with metadata fields (`expires_at`, `ttl_seconds`).

**Acceptance Scenarios**:

1. **Given** an OAuth2 credential with expiring access token, **When** `Credential::refresh()` is called, **Then** token is refreshed using refresh_token
2. **Given** credential metadata with `expires_at` and `ttl_seconds`, **When** rotation system checks expiration, **Then** it uses metadata fields without needing `TokenRefreshValidator` methods
3. **Given** removal of `TokenRefreshValidator` trait, **When** codebase is compiled, **Then** no references to the trait remain

---

### Edge Cases

- What happens when imports reference the old `traits/` module after it's deleted?
- How does the system handle existing credential implementations that import from `traits/`?
- What if `rotation/validation.rs` still has duplicate trait definitions after refactoring?
- How do we ensure no circular dependencies between `core/traits.rs` and `core/validation.rs`?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST consolidate all trait definitions (`Credential`, `TestableCredential`, `RotatableCredential`, `InteractiveCredential`) into `core/traits.rs`
- **FR-002**: System MUST move validation types (`ValidationContext`, `ValidationOutcome`, `ValidationFailureHandler`, `ValidationFailureType`) from `rotation/validation.rs` to `core/validation.rs`
- **FR-003**: System MUST remove duplicate trait definitions from `rotation/validation.rs` 
- **FR-004**: System MUST delete the `traits/` directory completely
- **FR-005**: System MUST remove `TokenRefreshValidator` trait as its functionality is covered by `Credential::refresh()` and `CredentialMetadata` fields
- **FR-006**: System MUST update all imports across the codebase to reference `core::traits` and `core::validation`
- **FR-007**: Codebase MUST compile without errors after refactoring
- **FR-008**: All existing tests MUST pass after refactoring

### Key Entities

- **core/traits.rs**: Single file containing all credential trait definitions (Credential, TestableCredential, RotatableCredential, InteractiveCredential)
- **core/validation.rs**: Single file containing all validation types (ValidationContext, ValidationOutcome, ValidationFailureHandler, ValidationFailureType, TestMethod, SuccessCriteria, ValidationTest)
- **rotation/validation.rs**: Reduced file containing only rotation-specific validation logic, no trait definitions

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All credential trait definitions exist in exactly one location (`core/traits.rs`)
- **SC-002**: All validation types exist in exactly one location (`core/validation.rs`)
- **SC-003**: Zero duplicate trait or type definitions across the codebase
- **SC-004**: `traits/` directory is completely removed
- **SC-005**: `TokenRefreshValidator` trait has zero references in the codebase
- **SC-006**: All imports use `core::traits` or `core::validation` paths
- **SC-007**: Codebase compiles successfully with `cargo check --workspace`
- **SC-008**: All tests pass with `cargo test --workspace`
- **SC-009**: Developer can implement a new credential type by importing only from `core` module
- **SC-010**: Code review confirms Rust best practices for module organization (traits in core, clear separation of concerns)

## Assumptions

- The existing `Credential::refresh()` method is sufficient for token refresh functionality
- `CredentialMetadata::expires_at` and `CredentialMetadata::ttl_seconds` provide all necessary expiration tracking
- Validation is a core concern of credentials, not specific to rotation
- Traits should live in the `core/` module following Rust conventions for foundational types
- Single-file organization (`core/traits.rs` and `core/validation.rs`) is acceptable given the moderate size of trait definitions

## Out of Scope

- Adding new traits or validation capabilities
- Modifying trait method signatures
- Changing validation logic or behavior
- Performance optimization
- Documentation updates beyond code comments
- Migration guide for external users (this is an internal refactoring)
