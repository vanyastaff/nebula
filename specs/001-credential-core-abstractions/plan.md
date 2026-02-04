# Implementation Plan: Core Credential Abstractions

**Branch**: `001-credential-core-abstractions` | **Date**: 2026-02-03 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/001-credential-core-abstractions/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Implement Phase 1 of nebula-credential roadmap: foundational traits, types, and abstractions for secure credential management including StorageProvider trait, Credential trait, AES-256-GCM encryption with Argon2id key derivation, SecretString type with automatic memory zeroization, and comprehensive error hierarchy. This phase establishes the core abstractions that all future phases (storage backends, rotation, protocols) will build upon.

**Current State**: Existing code in `crates/nebula-credential/src/` has basic structure (flows, traits, utils) but needs refactoring to match Phase 1 design:
- ✅ Has: SecureString (needs rename + API improvement), basic error types, credential flows
- ❌ Missing: StorageProvider trait, EncryptionKey, EncryptedData, CredentialId validation, separate error hierarchy
- 🔄 Breaking changes allowed: Improve existing code to match Phase 1 specification

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)  
**Primary Dependencies**: 
- tokio (v1.49+) - async runtime
- zeroize (v1.8+) - memory zeroization for secrets
- aes-gcm (v0.10+) - AES-256-GCM AEAD encryption
- argon2 (latest) - password-based key derivation
- subtle (v2.5+) - constant-time comparisons
- serde (v1.0+) - serialization
- async-trait (v0.1+) - async traits
- thiserror (v1.0+) - error definitions

**Storage**: File-based local storage with encrypted credentials (Phase 2 adds cloud providers)  
**Testing**: `cargo test --workspace`, `#[tokio::test(flavor = "multi_thread")]` for async tests, `tokio::time::pause()` for time-based key derivation tests  
**Target Platform**: Cross-platform (Windows, Linux, macOS) with platform-specific file permissions  
**Project Type**: Workspace (nebula-credential crate in Domain layer)  
**Performance Goals**: 
- Encryption/decryption: <5ms p95 latency
- Key derivation: 100-200ms (security requirement, not performance target)
- Local storage operations: <10ms p95 latency

**Constraints**: 
- Constant-time cryptographic operations (prevent timing attacks)
- Memory zeroization for all sensitive data
- No external service dependencies for core functionality
- File permissions: 0600 Unix, restricted ACLs Windows
- Rust 2024 edition (sized types in type aliases)

**Scale/Scope**: 
- Single-machine credential storage (no distributed coordination)
- Credential size limit: TBD (reasonable default like 64KB to prevent memory exhaustion)
- Concurrent access: thread-safe via Tokio sync primitives
- Storage capacity: filesystem-limited (no artificial limits)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Feature uses newtype patterns (CredentialId, SecretString), enums for errors, sized types (String not str), explicit type annotations for complex generics
- [x] **Isolated Error Handling**: nebula-credential defines its own error types using thiserror (CredentialError, StorageError, CryptoError) without depending on shared error crate
- [x] **Test-Driven Development**: Test strategy defined - write tests first for encryption, key derivation, storage operations, error handling, and memory zeroization
- [x] **Async Discipline**: StorageProvider trait uses async-trait, all operations support cancellation via tokio::select!, timeouts configured (5s read, 10s write), proper use of RwLock for caching layer in future
- [x] **Modular Architecture**: nebula-credential is in Domain layer, depends only on nebula-core (lower layer), no circular dependencies, provides abstractions for higher layers
- [x] **Observability**: All operations use tracing with context (credential_id, operation), errors logged with context before propagation, secrets redacted in logs, preparation for metrics in Phase 8
- [x] **Simplicity**: No premature abstractions - only core traits (Credential, StorageProvider) and essential types, complexity justified by security requirements (encryption, key derivation, zeroization)

**Gate Result**: ✅ PASS - All constitution principles satisfied

## Project Structure

### Documentation (this feature)

```text
specs/001-credential-core-abstractions/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
│   └── storage-provider-trait.md  # StorageProvider trait contract
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
crates/
├── nebula-credential/     # Primary crate for this feature (Domain layer)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── core/          # Core types (CredentialId, CredentialContext, CredentialMetadata)
│   │   │   ├── mod.rs
│   │   │   ├── error.rs   # Error hierarchy (CredentialError, StorageError, CryptoError)
│   │   │   ├── metadata.rs
│   │   │   ├── context.rs
│   │   │   ├── state.rs
│   │   │   └── result.rs
│   │   ├── traits/        # Core traits
│   │   │   ├── mod.rs
│   │   │   ├── credential.rs  # Credential trait
│   │   │   ├── storage.rs     # StorageProvider trait
│   │   │   └── lock.rs
│   │   ├── utils/         # Utility types
│   │   │   ├── mod.rs
│   │   │   ├── secure_string.rs  # SecretString with zeroization
│   │   │   ├── crypto.rs         # Encryption (AES-GCM, Argon2)
│   │   │   └── time.rs
│   │   └── flows/         # Existing (credential flows - not modified in Phase 1)
│   ├── tests/             # Integration tests
│   │   ├── encryption_tests.rs
│   │   ├── storage_trait_tests.rs
│   │   ├── error_tests.rs
│   │   └── zeroization_tests.rs
│   ├── examples/          # Usage examples
│   │   └── basic_credential_storage.rs
│   └── Cargo.toml
└── nebula-core/           # Dependency (provides Id types, scope system)

# Workspace layers:
# Infrastructure: nebula-log, nebula-config
# Cross-Cutting: nebula-core, nebula-value
# Domain: nebula-credential (THIS FEATURE), nebula-parameter, nebula-action, nebula-expression, nebula-validator
# UI: nebula-ui, nebula-parameter-ui
# System: nebula-memory, nebula-resilience, nebula-resource, nebula-system
# Tooling: nebula-derive
```

**Structure Decision**: This feature modifies the existing `nebula-credential` crate in the Domain layer. No new crates are created. The crate already exists with basic structure (core/, traits/, utils/, flows/) but lacks complete Phase 1 implementation. This aligns with Constitution Principle V (Modular Architecture) by keeping credential management in a single focused crate that depends only on lower-layer crates (nebula-core for Id types).

The Domain layer is appropriate because credentials are core business domain concepts (similar to parameters, actions, expressions) rather than infrastructure concerns (logging, config) or UI concerns.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

No violations - Constitution Check passed all principles. Complexity is inherent to security requirements (cryptography, memory zeroization) and cannot be simplified without compromising security.

## Phase 0: Research & Discovery ✅ COMPLETE

**See**: [research.md](./research.md) for complete findings

### Research Tasks (Completed)

1. **AES-256-GCM Implementation Patterns** ✅
   - Decision: Counter-based nonce with AtomicU64 (prevents reuse, 2^64 capacity)
   - Decision: Constant-time tag comparison using subtle::ConstantTimeEq
   - Source: RustCrypto/AEADs documentation via DeepWiki MCP

2. **Argon2id Key Derivation** ✅
   - Decision: 19 MiB memory cost, 2 iterations, 32-byte output
   - Rationale: OWASP 2024 recommendations, 100-200ms derivation time
   - Source: argon2 crate documentation via Context7 MCP

3. **Memory Zeroization Patterns** ✅
   - Decision: SecretString with ZeroizeOnDrop + expose_secret() closure API
   - Rationale: Prevents accidental secret copying, automatic memory clearing
   - Current code: SecureString with expose() - needs API improvement

4. **Async Storage Trait Design** ✅
   - Decision: async-trait for StorageProvider (compatibility with Rust 1.92)
   - Rationale: Simpler than manual futures, widely adopted pattern
   - Current code: Missing StorageProvider trait - needs implementation

5. **Error Hierarchy Design** ✅
   - Decision: Three-tier with thiserror (CredentialError → Storage/Crypto/Validation)
   - Current code: Monolithic CredentialError - needs refactoring into separate types

### Decisions Made

**Decision 1: Nonce Management Strategy**
- **Chosen**: Counter-based nonce with atomic operations (AtomicU64)
- **Rationale**: Prevents nonce reuse (critical for AES-GCM security), deterministic, no external randomness per-operation overhead
- **Alternatives Rejected**:
  - Pure random nonces: Risk of collision at scale
  - File-based counter: I/O overhead on every encryption
- **Reference**: RustCrypto/AEADs documentation on nonce uniqueness requirements

**Decision 2: SecretString Exposure API**
- **Chosen**: `expose_secret<F, R>(&self, f: F) -> R where F: FnOnce(&str) -> R`
- **Rationale**: Forces callers to use closure scope, prevents accidental secret copying
- **Alternatives Rejected**:
  - Direct getter: Allows uncontrolled secret propagation
  - Deref implementation: Makes secrets too easy to leak
- **Reference**: Existing patterns in nebula-credential SecretString

**Decision 3: Storage Provider Trait Async Design**
- **Chosen**: async-trait with `async fn` methods
- **Rationale**: Enables cloud provider implementations (AWS, Azure, Vault) in Phase 2
- **Alternatives Rejected**:
  - Sync trait: Blocks Phase 2 cloud providers
  - Return Future manually: Complex, verbose, unnecessary
- **Reference**: Tokio async-trait patterns

**Decision 4: Credential ID Validation**
- **Chosen**: Regex validation at CredentialId::new() - alphanumeric + hyphens + underscores
- **Rationale**: Prevents path traversal, filesystem issues, injection attacks
- **Alternatives Rejected**:
  - No validation: Security risk
  - UUID-only: Too restrictive for user-defined IDs
- **Reference**: OWASP Input Validation guidelines

**Decision 5: Encryption Algorithm Versioning**
- **Chosen**: Include version byte in EncryptedData structure (u8 version field)
- **Rationale**: Enables future algorithm migrations (post-quantum crypto)
- **Alternatives Rejected**:
  - No versioning: Future migrations require breaking changes
  - Separate metadata file: Overhead, complexity
- **Reference**: Future Consideration FC-001 from spec

## Phase 1: Architecture & Contracts ✅ COMPLETE

**See**: [data-model.md](./data-model.md) for complete type definitions

### Data Model

**Core Types** (to implement/refactor):
- `CredentialId`: Newtype over String with validation (NEW - add validation logic)
- `SecretString`: Rename from SecureString, improve API with `expose_secret()` closure
- `EncryptionKey`: 256-bit AES key with automatic zeroization (NEW)
- `EncryptedData`: Container with ciphertext, nonce, tag, version (NEW)
- `CredentialMetadata`: Exists, may need updates for Phase 1 requirements
- `CredentialContext`: Exists, may need updates for Phase 1 requirements

**Trait Definitions**:
- `Credential`: Exists in traits/credential.rs, verify matches spec
- `StorageProvider`: NEW - async trait for persistence operations (Phase 1 core addition)

**Error Types** (needs refactoring):
- `CredentialError`: Top-level error enum (REFACTOR - current is too broad)
- `StorageError`: NEW - File I/O, permissions, not found (split from CredentialError)
- `CryptoError`: NEW - Encryption, decryption, key derivation (split from CredentialError)
- `ValidationError`: NEW - Invalid credential IDs, malformed data (split from CredentialError)

### API Contracts

**See**: [contracts/storage-provider-trait.md](./contracts/storage-provider-trait.md) for complete StorageProvider trait definition

**Core Operations**:
1. `store(id, encrypted_data, metadata, context) -> Result<()>`
2. `retrieve(id, context) -> Result<EncryptedData>`
3. `delete(id, context) -> Result<()>`
4. `list(filter, context) -> Result<Vec<CredentialId>>`
5. `exists(id, context) -> Result<bool>`

**Error Handling**: All operations return `Result<T, StorageError>` with context-rich errors

### Quick Start Guide

**See**: [quickstart.md](./quickstart.md) for developer quick start

**10-Line Example**:
```rust
use nebula_credential::{CredentialId, SecretString, EncryptionKey};

let key = EncryptionKey::derive_from_password("master-pwd", &salt)?;
let secret = SecretString::new("api-key-12345");
let id = CredentialId::new("github_token")?;
// Store and retrieve via provider (Phase 2)
```

## Phase 2: Task Breakdown

**Status**: Ready for `/speckit.tasks` command

### Implementation Strategy

Given existing code in `crates/nebula-credential/src/`, the implementation will:

1. **Refactor existing code** (breaking changes allowed):
   - Rename `SecureString` → `SecretString` with improved API
   - Split monolithic `CredentialError` into separate error types
   - Add validation to existing types

2. **Add new Phase 1 types**:
   - `CredentialId` with validation
   - `EncryptionKey` with Argon2id derivation
   - `EncryptedData` with version field
   - `StorageProvider` trait

3. **Add crypto module** (new):
   - AES-256-GCM encryption/decryption
   - Nonce generator with AtomicU64
   - Key derivation from password

4. **Update existing traits**:
   - Verify `Credential` trait matches spec
   - Update error handling throughout

### File Changes Required

**Note**: Structure папок **НЕ меняется** - уже правильная! Работаем с существующими файлами.

```
crates/nebula-credential/src/
├── core/
│   ├── error.rs          # REFACTOR: Split into Storage/Crypto/Validation errors
│   ├── mod.rs            # UPDATE: Export new types (CredentialId, etc.)
│   ├── context.rs        # REVIEW: Verify matches spec
│   ├── metadata.rs       # REVIEW: Verify matches spec
│   └── [other existing]  # KEEP: adapter.rs, result.rs, state.rs for flows
├── traits/
│   ├── storage.rs        # EXISTS! UPDATE: Add/verify StorageProvider trait
│   ├── credential.rs     # EXISTS! REVIEW: Verify matches spec
│   ├── lock.rs           # EXISTS! KEEP: For Phase 4 (distributed locking)
│   └── mod.rs            # UPDATE: Export StorageProvider if needed
├── utils/
│   ├── secure_string.rs  # EXISTS! REFACTOR: Rename to secret_string.rs, improve API
│   ├── crypto.rs         # EXISTS! UPDATE: Add encryption/decryption, key derivation
│   ├── time.rs           # EXISTS! KEEP: Time utilities
│   └── mod.rs            # UPDATE: Export new crypto functions
├── flows/                # EXISTS! KEEP: OAuth2, API key flows (Phase 5)
│   ├── oauth2/           # KEEP: For Phase 5
│   └── [others]          # KEEP: For Phase 5
└── lib.rs                # UPDATE: Update public exports for Phase 1

tests/
├── encryption_tests.rs   # NEW: Crypto tests (AES-GCM, Argon2id)
├── storage_trait_tests.rs # NEW: StorageProvider trait tests
├── error_tests.rs        # UPDATE: Test new error hierarchy
└── zeroization_tests.rs  # NEW: Memory zeroization tests
```

**Структура уже оптимальна** - не требует изменений, только наполнение контентом Phase 1.

## Future-Proofing (Extensibility Design)

**ВАЖНО**: Phase 1 создаёт фундамент, но **НЕ реализует** всё из docs. Архитектура из `docs/Meta/ARCHITECTURE-DESIGN.md` - это **видение на phases 1-10**, а Phase 1 - только база.

### What Phase 1 DOES (Minimal Foundation)

✅ **Implement now:**
- `Credential` trait (базовый, существующий в `traits/credential.rs`)
- `StorageProvider` trait (уже есть в `traits/storage.rs` - обновить под spec)
- Core types: `CredentialId`, `SecretString`, `EncryptionKey`, `EncryptedData`
- Error types: `StorageError`, `CryptoError`, `ValidationError`
- Basic context: `CredentialContext` с `owner_id`, `scope_id: Option<String>`, `trace_id`

### What Phase 1 DOES NOT (Future Phases)

❌ **NOT in Phase 1:**
- `InteractiveCredential` trait → Phase 5 (OAuth2, SAML flows)
- `RotatableCredential` trait → Phase 4 (rotation logic)
- `CredentialProtocol` marker trait → Phase 5 (protocol system)
- Type-state pattern (`OAuth2Flow<State>`) → Phase 5
- State machine (`CredentialState` enum) → Phase 4
- `CredentialScope` with tenant isolation → Phase 6 (multi-tenancy)
- Distributed locking → Phase 4
- Audit logging → Phase 7

### Trait Hierarchy Vision (Future Reference Only)

```rust
// ✅ Phase 1: Implement base trait (EXISTS in traits/credential.rs - verify/update)
pub trait Credential: Send + Sync + 'static {
    type Output: Send + Sync;
    async fn authenticate(&self, ctx: &CredentialContext) -> Result<Self::Output>;
}

// ❌ Phase 5: Will add InteractiveCredential later (NOT Phase 1)
// pub trait InteractiveCredential: Credential { /* ... */ }

// ❌ Phase 4: Will add RotatableCredential later (NOT Phase 1)
// pub trait RotatableCredential: Credential { /* ... */ }
```

### Type-State Pattern (Prepared, Not Implemented)

✅ Phase 1 создаёт типы, готовые для type-state в Phase 5:

```rust
// ✅ Phase 1: Version field allows future algorithm transitions
pub struct EncryptedData { 
    pub version: u8,  // ← Enables version-based type-state in future
    pub ciphertext: Vec<u8>, 
    /* ... */ 
}

// ❌ Phase 5 will add: Type-state for OAuth2 flow (NOT Phase 1)
// struct OAuth2Flow<State> { /* ... */ }
```

**Design decision**: Phase 1 types have fields (like `version`, `scope_id: Option<String>`) that make future extensions **non-breaking**.

### Protocol Extension Points (Generic by Design)

✅ Phase 1 `StorageProvider` generic enough для всех будущих протоколов:

```rust
// ✅ Phase 1: Store любые encrypted credentials (не знает о конкретных протоколах)
#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn store(&self, id: &CredentialId, data: EncryptedData, /* ... */);
    // Phase 2+ backends: Local, AWS, Azure, Vault, K8s
    // Phase 5+ data: OAuth2Token, JWTToken, SAMLAssertion - все через EncryptedData!
}

// ❌ Phase 5 will add: Protocol marker trait (NOT Phase 1)
// pub trait CredentialProtocol: Send + Sync + 'static { /* ... */ }
```

**Design decision**: `EncryptedData` is opaque blob - storage не зависит от protocol-specific types.

### Scope Isolation (Simple Now, Extensible Later)

✅ Phase 1 `CredentialContext` minimally functional, extensible для Phase 6:

```rust
// ✅ Phase 1: Simple context (implement now)
pub struct CredentialContext {
    pub owner_id: String,              // Who owns credential
    pub scope_id: Option<String>,      // ← Optional now, Phase 6 will use this
    pub trace_id: Uuid,                // For observability
    pub timestamp: DateTime<Utc>,
}

// ❌ Phase 6 will add: Full scope isolation type (NOT Phase 1)
// pub struct CredentialScope { tenant_id, workflow_id, user_id, tags }
```

**Design decision**: `scope_id: Option<String>` is extension point - Phase 1 doesn't enforce scope rules, Phase 6 will.

### State Machine (Phase 1 → Phase 4/5)

Phase 1 `CredentialMetadata` has `RotationPolicy` stub:

```rust
// Phase 1: Metadata with rotation stub
pub struct CredentialMetadata {
    pub created_at: DateTime<Utc>,
    pub rotation_policy: Option<RotationPolicy>, // ← Stub for Phase 4
}

// Phase 4: Full state machine
pub enum CredentialState {
    Uninitialized, PendingInteraction, Authenticating,
    Active, Expired, Refreshing, RotationScheduled, Rotating, GracePeriod,
    Revoked, Failed,
}
```

### Error Extensibility (Phase 1 → All Phases)

Phase 1 error hierarchy designed for extension:

```rust
// Phase 1: Core errors
pub enum CredentialError {
    Storage { source: StorageError },
    Crypto { source: CryptoError },
    Validation { source: ValidationError },
}

// Phase 7: Add audit errors without breaking changes
pub enum CredentialError {
    Storage { source: StorageError },
    Crypto { source: CryptoError },
    Validation { source: ValidationError },
    Audit { source: AuditError },      // ← New in Phase 7
}
```

### Design Principles Applied

1. **Open/Closed Principle**: Phase 1 traits open for extension (InteractiveCredential, RotatableCredential), closed for modification
2. **Dependency Inversion**: `StorageProvider` trait allows Phase 2 to add AWS/Azure/Vault without changing Phase 1 code
3. **Single Responsibility**: Each type has one job (EncryptionKey encrypts, StorageProvider stores, CredentialContext carries context)
4. **Interface Segregation**: Small focused traits (not one giant Credential trait with all methods)

## Next Steps

1. **✅ Planning complete** - Technical decisions validated, architecture designed for phases 1-10
2. **Run `/speckit.tasks`** - Generate detailed task breakdown in tasks.md with TDD workflow
3. **Run `/speckit.implement`** - Execute implementation following tasks.md
4. **Breaking changes OK** - Refactor existing code to match extensible Phase 1 design
5. **Keep docs in mind** - Design decisions guided by `docs/Meta/ARCHITECTURE-DESIGN.md`

## References

- Feature Specification: [spec.md](./spec.md)
- Roadmap: `crates/nebula-credential/docs/ROADMAP.md` Phase 1
- Architecture: `crates/nebula-credential/docs/Meta/ARCHITECTURE-DESIGN.md`
- Technical Design: `crates/nebula-credential/docs/Meta/TECHNICAL-DESIGN.md`
- Constitution: `.specify/memory/constitution.md`
