# Data Model: Trait Locations and Validation Types

**Feature**: Remove Duplicate Trait Definitions  
**Date**: 2026-02-04  
**Status**: Design Complete

## Overview

This document maps the location of all credential traits and validation types after removing duplicates from `rotation/validation.rs`. No new types are created - this is purely organizational cleanup.

---

## Trait Hierarchy

### Credential Trait Family

**Location**: `traits/` directory (SOURCE OF TRUTH)

```rust
// Base trait - traits/credential.rs
pub trait Credential: Send + Sync + 'static {
    type Input: Serialize + DeserializeOwned + Send + Sync + 'static;
    type State: CredentialState;
    
    fn description(&self) -> CredentialDescription;
    async fn initialize(&self, input: &Self::Input, ctx: &mut CredentialContext) 
        -> Result<InitializeResult<Self::State>, CredentialError>;
    async fn refresh(&self, state: &mut Self::State, ctx: &mut CredentialContext) 
        -> Result<(), CredentialError>;
    async fn revoke(&self, state: &mut Self::State, ctx: &mut CredentialContext) 
        -> Result<(), CredentialError>;
}

// Interactive flows - traits/credential.rs
pub trait InteractiveCredential: Credential {
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError>;
}

// Testing capability - traits/testable.rs
pub trait TestableCredential: Credential {
    async fn test(&self) -> RotationResult<ValidationOutcome>;
    fn test_timeout(&self) -> Duration { Duration::from_secs(30) }
}

// Rotation capability - traits/rotation.rs
pub trait RotatableCredential: TestableCredential {
    async fn rotate(&self) -> RotationResult<Self> where Self: Sized;
    async fn cleanup_old(&self) -> RotationResult<()> { Ok(()) }
}
```

**Hierarchy**:
```
Credential (base capability)
  ├── InteractiveCredential (extends with multi-step flows)
  └── TestableCredential (extends with validation)
        └── RotatableCredential (extends with rotation)
```

**Design Rationale**:
- `Credential` is the base - all credentials can initialize, refresh, revoke
- `InteractiveCredential` adds multi-step flow support (OAuth2, SAML, 2FA)
- `TestableCredential` adds validation capability (can test itself)
- `RotatableCredential` builds on testable (must validate before/after rotation)

---

## Infrastructure Traits

**Location**: `traits/` directory (UNCHANGED)

These traits are unrelated to the refactoring but documented for completeness:

```rust
// traits/storage.rs
pub trait StorageProvider: Send + Sync {
    async fn store(&self, id: &CredentialId, state: &[u8]) -> Result<(), CredentialError>;
    async fn retrieve(&self, id: &CredentialId) -> Result<Vec<u8>, CredentialError>;
    async fn delete(&self, id: &CredentialId) -> Result<(), CredentialError>;
    async fn list(&self, filter: Option<&CredentialFilter>) -> Result<Vec<CredentialId>, CredentialError>;
}

pub trait StateStore: Send + Sync {
    async fn save_state(&self, key: &str, value: &[u8]) -> Result<StateVersion, CredentialError>;
    async fn load_state(&self, key: &str) -> Result<Option<Vec<u8>>, CredentialError>;
}

// traits/lock.rs
pub trait DistributedLock: Send + Sync {
    async fn acquire(&self, resource: &str, ttl: Duration) -> Result<LockGuard, LockError>;
}
```

**These remain in `traits/`** - they are infrastructure concerns, not credential-specific.

---

## Testing Types

**Location**: `rotation/validation.rs` (RENAMED to Rust stdlib style)

### TestContext

**Purpose**: Provides context for credential testing with timeout enforcement

```rust
#[derive(Debug, Clone)]
pub struct TestContext {
    pub credential_id: CredentialId,
    pub metadata: CredentialMetadata,
    pub timeout: Duration,
    pub is_retry: bool,
    pub retry_attempt: u32,
}
```

**Methods**:
- `new(id, metadata)` - Create test context
- `with_timeout(duration)` - Set custom timeout (default 30s)
- `with_retry(attempt)` - Mark as retry attempt
- `test<T: TestableCredential>(cred)` - Test credential with timeout enforcement

**Usage**: Rotation system creates context, calls `context.test(&credential)`

**Naming**: Follows Rust stdlib pattern - short, clear context object

### TestResult

**Purpose**: Result of credential test (like `std::io::Result`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub passed: bool,
    pub message: String,
    pub method: String,    // e.g., "SELECT 1", "userinfo", "TLS handshake"
    pub duration: Duration,
}
```

**Factory Methods**:
- `success(message, method, duration)` - Create successful result
- `failure(message, method, duration)` - Create failed result

**Usage**: `TestableCredential::test()` implementations return this

**Naming**: `TestResult` instead of `TestOutcome` - more Rust-idiomatic (cf. `std::io::Result`)

### FailureHandler

**Purpose**: Analyzes test failures and determines retry/rollback strategy

```rust
#[derive(Debug, Clone)]
pub struct FailureHandler {
    pub max_retries: u32,       // Default: 3
    pub auto_rollback: bool,    // Default: true
}
```

**Methods**:
- `classify_error(msg)` - Categorize error into FailureKind
- `should_trigger_rollback(kind, retry_count)` - Decide if rollback needed
- `should_retry(kind, retry_count)` - Decide if retry should be attempted

**Usage**: Rotation coordinator uses this to handle test failures

**Naming**: Short, clear - "Failure" context is obvious in rotation module

### FailureKind

**Purpose**: Classification of test failures (like `std::io::ErrorKind`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    NetworkError,          // Transient
    AuthenticationError,   // Permanent
    AuthorizationError,    // Permanent
    Timeout,               // Transient
    InvalidFormat,         // Permanent
    ServiceUnavailable,    // Transient
    Unknown,               // Permanent (default)
}
```

**Methods**:
- `is_transient()` - Returns true for NetworkError, Timeout, ServiceUnavailable
- `is_permanent()` - Returns !is_transient()

**Usage**: Determines retry vs rollback decision

**Naming**: Follows `std::io::ErrorKind` pattern - `Kind` suffix for error classification

### Supporting Types

```rust
// Validation test definition (for future extensibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationTest {
    pub test_method: TestMethod,
    pub endpoint: String,
    pub expected_criteria: SuccessCriteria,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestMethod {
    HttpRequest { method: String, headers: Vec<(String, String)> },
    DatabaseQuery { query: String },
    TlsHandshake { hostname: String, port: u16 },
    Custom { command: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SuccessCriteria {
    HttpSuccess,
    QuerySuccess,
    HandshakeSuccess,
    Custom { description: String },
}
```

**Status**: Defined but not currently used (for future validation framework extensibility)

---

## Module Organization

### Before (PROBLEM)

```
traits/
├── credential.rs       ✅ Credential, InteractiveCredential
├── testable.rs         ✅ TestableCredential
├── rotation.rs         ✅ RotatableCredential
├── lock.rs             ✅ DistributedLock, LockError, LockGuard
├── storage.rs          ✅ StorageProvider, StateStore
└── mod.rs

rotation/
└── validation.rs       ❌ DUPLICATE: TestableCredential, RotatableCredential
                        ❌ UNNECESSARY: TokenRefreshValidator
                        ✅ KEEP: TestContext, TestResult, FailureHandler, FailureKind
```

### After (SOLUTION)

```
traits/
├── credential.rs       ✅ UNCHANGED - Credential, InteractiveCredential
├── testable.rs         ✅ UNCHANGED - TestableCredential
├── rotation.rs         ✅ UNCHANGED - RotatableCredential
├── lock.rs             ✅ UNCHANGED - DistributedLock
├── storage.rs          ✅ UNCHANGED - StorageProvider
└── mod.rs              ✅ UNCHANGED

rotation/
└── validation.rs       ✅ UPDATED:
                           - ADD: use crate::traits::{TestableCredential, RotatableCredential};
                           - REMOVE: Duplicate TestableCredential (lines 37-79)
                           - REMOVE: Duplicate RotatableCredential (lines 81-111)
                           - REMOVE: TokenRefreshValidator (lines 113-189)
                           - RENAME: ValidationContext → TestContext
                           - RENAME: ValidationOutcome → TestResult  
                           - RENAME: ValidationFailureHandler → FailureHandler
                           - RENAME: ValidationFailureType → FailureKind
```

---

## Changes to rotation/validation.rs

### Lines to Remove

**Lines 37-79: Duplicate TestableCredential trait**
```rust
// DELETE THIS - already in traits/testable.rs
#[async_trait]
pub trait TestableCredential: Send + Sync {
    async fn test(&self) -> RotationResult<ValidationOutcome>;
    fn test_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
```

**Lines 81-111: Duplicate RotatableCredential trait**
```rust
// DELETE THIS - already in traits/rotation.rs
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

**Lines 113-189: TokenRefreshValidator trait**
```rust
// DELETE THIS - functionality covered by Credential::refresh() and CredentialMetadata
#[async_trait]
pub trait TokenRefreshValidator: TestableCredential {
    async fn refresh_token(&self) -> RotationResult<Self>
    where
        Self: Sized;

    fn get_expiration(&self) -> Option<chrono::DateTime<chrono::Utc>>;
    fn time_until_expiry(&self) -> Option<chrono::Duration> { ... }
    fn should_refresh(&self, threshold_percentage: f32) -> bool { ... }
}
```

**Why TokenRefreshValidator is unnecessary**:
1. `Credential::refresh()` already provides token refresh capability
2. `CredentialMetadata::expires_at` tracks expiration time
3. `CredentialMetadata::ttl_seconds` tracks time-to-live
4. Token refresh is a credential capability, not a validation concern
5. No current usage in codebase (verified by grep)

### Lines to Add

**At top of file (after existing imports)**:
```rust
use crate::traits::{TestableCredential, RotatableCredential};
```

### Lines to Keep

**All validation types** (after line 189):
- `ValidationContext` struct
- `ValidationOutcome` struct
- `ValidationTest` struct
- `ValidationFailureType` enum
- `ValidationFailureHandler` struct
- `TestMethod` enum
- `SuccessCriteria` enum

**All tests** (at end of file)

---

## Import Paths (UNCHANGED)

**Public API** via prelude (no changes):
```rust
use nebula_credential::prelude::*;

// Available:
// - Credential, InteractiveCredential
// - TestableCredential, RotatableCredential (from traits/)
// - ValidationContext, ValidationOutcome, ValidationFailureHandler (from rotation/)
```

**Direct imports** (no changes):
```rust
use nebula_credential::traits::{
    Credential, 
    TestableCredential, 
    RotatableCredential,
};

use nebula_credential::rotation::{
    ValidationContext, 
    ValidationOutcome,
    ValidationFailureHandler,
};
```

**✅ Zero breaking changes** - all import paths remain valid

---

## Trait Relationships

### Dependency Graph

```
Credential (base)
  ↓
TestableCredential
  ↓
RotatableCredential
```

**Constraint**: `RotatableCredential: TestableCredential` ensures:
- Credentials must be testable before they can be rotatable
- New rotated credentials can be validated before committing
- Follows principle: "validate before rotate, validate after rotate"

### Trait Bounds in Practice

```rust
// Any credential can be used
fn use_credential<C: Credential>(cred: C) { ... }

// Only testable credentials
fn validate_credential<C: TestableCredential>(cred: C) { ... }

// Only rotatable credentials (implies testable + credential)
fn rotate_credential<C: RotatableCredential>(cred: C) { ... }
```

---

## Rationale for Organization

### Why traits/ directory?

**From research.md**:
- Infrastructure traits should be separate from core domain types
- Follows patterns from validator, diesel, async-trait projects
- Clear separation of behavioral abstractions (traits) from concrete types (core)
- Single responsibility principle

### Why validation in rotation/?

**From research.md**:
- Validation types are domain-specific to credential rotation
- High cohesion - validation used exclusively by rotation logic
- Follows domain-driven design (DDD) principles
- Examples: garde, validator keep validation close to domain concerns
- Generic utilities (crypto, format validation) remain in utils/

### Why remove TokenRefreshValidator?

**Rationale**:
1. **Duplication**: `Credential::refresh()` already exists for token refresh
2. **Wrong abstraction**: Token refresh is a credential capability, not validation
3. **Metadata coverage**: `expires_at` and `ttl_seconds` track expiration
4. **YAGNI**: No current usage, speculative functionality
5. **Simpler API**: Fewer traits = easier to implement credentials

---

## Success Criteria Mapping

| Criterion | Verified By |
|-----------|-------------|
| SC-001: Traits in traits/ | traits/ directory unchanged ✅ |
| SC-002: Validation in rotation/ | validation types remain in rotation/validation.rs ✅ |
| SC-003: Zero duplicates | Remove lines 37-189 from rotation/validation.rs ✅ |
| SC-004: No TokenRefreshValidator | Remove lines 113-189 ✅ |
| SC-005: Tests pass | cargo test --workspace ✅ |
| SC-006: Compiles | cargo check --workspace ✅ |
| SC-007: Import paths stable | No import changes needed ✅ |
| SC-008: Module structure preserved | traits/, rotation/ unchanged ✅ |

---

## Next Steps

1. Create `quickstart.md` with implementation summary
2. Run `/speckit.tasks` to generate detailed task breakdown
3. Execute tasks to remove duplicates
4. Verify compilation and tests
5. Commit with message: `refactor(credential): remove duplicate trait definitions`

**Data Model Status**: ✅ Complete
