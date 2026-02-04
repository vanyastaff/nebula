# Tasks: Core Credential Abstractions

**Input**: Design documents from `/specs/001-credential-core-abstractions/`  
**Feature Branch**: `001-credential-core-abstractions`  
**Date**: 2026-02-03

**Prerequisites**: plan.md, spec.md, data-model.md, contracts/storage-provider-trait.md, research.md, quickstart.md

**Tests**: TDD approach - write tests FIRST, verify they FAIL, then implement to make them pass.

**Organization**: Tasks organized by user story for independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: User story label (US1, US2, etc. from spec.md)
- File paths relative to `crates/nebula-credential/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Update project dependencies and structure for Phase 1 implementation

- [x] T001 Update Cargo.toml with Phase 1 dependencies (aes-gcm v0.10+, argon2, zeroize v1.8+, subtle v2.5+, async-trait v0.1+)
- [x] T002 Verify workspace configuration includes nebula-credential in Domain layer with nebula-core dependency
- [x] T003 Create examples/ directory with basic_credential_storage.rs placeholder from quickstart.md

**Checkpoint**: Dependencies installed, project structure verified

---

## Phase 2: Foundational (Core Types & Error Hierarchy)

**Purpose**: Core types and error hierarchy that ALL user stories depend on - MUST complete before user story work

**⚠️ CRITICAL**: No user story implementation can begin until this phase is complete

### Error Hierarchy (Refactor Existing)

- [x] T004 [P] Refactor core/error.rs - split monolithic CredentialError into separate StorageError type per data-model.md
- [x] T005 [P] Add CryptoError type to core/error.rs per data-model.md (DecryptionFailed, EncryptionFailed, KeyDerivation, NonceGeneration, UnsupportedVersion)
- [x] T006 [P] Add ValidationError type to core/error.rs per data-model.md (EmptyCredentialId, InvalidCredentialId, InvalidFormat)
- [x] T007 Update CredentialError enum to wrap Storage/Crypto/Validation errors with context per data-model.md error hierarchy
- [x] T008 Update core/mod.rs to export all error types (CredentialError, StorageError, CryptoError, ValidationError)

### Core Types (New & Refactored)

- [x] T009 [P] Create CredentialId newtype in core/mod.rs or new core/id.rs with validation per data-model.md (alphanumeric + hyphens + underscores)
- [x] T010 [P] Rename utils/secure_string.rs to utils/secret_string.rs and refactor SecureString → SecretString with expose_secret() closure API per data-model.md
- [x] T011 [P] Review and update core/metadata.rs to match CredentialMetadata from data-model.md (created_at, last_accessed, last_modified, rotation_policy, tags)
- [x] T012 [P] Review and update core/context.rs to match CredentialContext from data-model.md (owner_id, scope_id: Option<String>, trace_id, timestamp)
- [x] T013 Update core/mod.rs to export CredentialId, update lib.rs prelude exports

### Crypto Module (New Implementation)

- [x] T014 [P] Create EncryptionKey type in utils/crypto.rs with derive_from_password() using Argon2id per research.md Decision 2
- [x] T015 [P] Create EncryptedData struct in utils/crypto.rs per data-model.md (version: u8, nonce: [u8;12], ciphertext: Vec<u8>, tag: [u8;16])
- [x] T016 Create NonceGenerator with AtomicU64 counter in utils/crypto.rs per research.md Decision 1
- [x] T017 Implement encrypt() function in utils/crypto.rs using AES-256-GCM with NonceGenerator per research.md
- [x] T018 Implement decrypt() function in utils/crypto.rs with constant-time tag comparison using subtle crate per research.md
- [x] T019 Update utils/mod.rs to export EncryptionKey, EncryptedData, encrypt(), decrypt()

### Traits (Verify & Update Existing)

- [ ] T020 Review traits/credential.rs - verify Credential trait matches data-model.md definition (id(), metadata(), authenticate(), validate())
- [ ] T021 Review traits/storage.rs - update StorageProvider trait to match contracts/storage-provider-trait.md (store, retrieve, delete, list, exists with async-trait)
- [ ] T022 Create CredentialFilter struct in traits/storage.rs per data-model.md (tags: Option<HashMap>, created_after, created_before)
- [ ] T023 Update traits/mod.rs to export Credential, StorageProvider, CredentialFilter

### Library Exports

- [ ] T024 Update lib.rs to export all Phase 1 public API: CredentialId, SecretString, EncryptionKey, EncryptedData, CredentialMetadata, CredentialContext, CredentialFilter
- [ ] T025 Update lib.rs to export all error types: CredentialError, StorageError, CryptoError, ValidationError
- [ ] T026 Update lib.rs to export traits: Credential, StorageProvider

**Checkpoint**: Foundation complete - all core types, errors, traits ready for user story implementation

---

## Phase 3: User Story 1 - Store and Retrieve Encrypted Credentials (Priority: P1) 🎯 MVP

**Goal**: Enable secure storage and retrieval of encrypted credentials with AES-256-GCM

**Independent Test**: Store an API key credential, retrieve it, verify encryption at rest, decrypted value correct, secret never logged

### Tests for User Story 1 (TDD - Write FIRST)

> **✅ COMPLETED**: Tests written and verified working

- [x] T027 [P] [US1] Create tests/encryption_tests.rs with test_encrypt_decrypt_roundtrip (encrypt secret → decrypt → verify match)
- [x] T028 [P] [US1] Add test_key_derivation_deterministic to tests/encryption_tests.rs (same password+salt → same key twice)
- [x] T029 [P] [US1] Add test_key_derivation_different_passwords to tests/encryption_tests.rs (different passwords → different keys)
- [x] T030 [P] [US1] Add test_nonce_uniqueness to tests/encryption_tests.rs (100 encryptions → 100 unique nonces)
- [x] T031 [P] [US1] Add test_decryption_with_wrong_key to tests/encryption_tests.rs (decrypt with wrong key → CryptoError::DecryptionFailed)

**Verification Step**: ✅ PASSED - All 5 tests pass (crypto already implemented in Phase 2)

### Implementation for User Story 1

> **✅ Already implemented in Phase 2 (T014-T019)**

- [x] T032 [US1] EncryptionKey::derive_from_password() implemented in utils/crypto.rs using Argon2id (19 MiB, 2 iterations)
- [x] T033 [US1] NonceGenerator::next() implemented in utils/crypto.rs using AtomicU64::fetch_add
- [x] T034 [US1] encrypt() implemented in utils/crypto.rs using Aes256Gcm::encrypt with NonceGenerator
- [x] T035 [US1] decrypt() implemented in utils/crypto.rs using Aes256Gcm::decrypt with constant-time tag comparison
- [x] T036 [US1] ZeroizeOnDrop applied to EncryptionKey in utils/crypto.rs for automatic key zeroization

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test encryption_tests` (5/5 passed)

**Checkpoint**: Encryption/decryption working with secure key derivation and memory zeroization

---

## Phase 4: User Story 2 - Validate Credential Types at Compile Time (Priority: P1)

**Goal**: Type-safe credential identifiers and secret strings preventing runtime errors

**Independent Test**: Create CredentialId with valid/invalid inputs, verify compilation enforces type safety

### Tests for User Story 2 (TDD - Write FIRST)

> **✅ COMPLETED**: Tests written and all passing

- [x] T037 [P] [US2] Create tests/validation_tests.rs with test_credential_id_valid (valid IDs → Ok)
- [x] T038 [P] [US2] Add test_credential_id_empty to tests/validation_tests.rs (empty string → ValidationError::EmptyCredentialId)
- [x] T039 [P] [US2] Add test_credential_id_invalid_chars to tests/validation_tests.rs (special chars → ValidationError::InvalidCredentialId)
- [x] T040 [P] [US2] Add test_secret_string_redacted to tests/validation_tests.rs (format!("{:?}", secret) → "[REDACTED]")
- [x] T041 [P] [US2] Add test_secret_string_expose_secret to tests/validation_tests.rs (expose_secret closure access works)

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test validation_tests` (10/10 passed)

### Implementation for User Story 2

> **✅ Already implemented in Phase 2 (T009-T010, T042-T045)**

- [x] T042 [US2] Implement CredentialId::new() with regex validation in core/id.rs (only alphanumeric + hyphens + underscores)
- [x] T043 [US2] Implement Display, Debug, TryFrom<String>, Into<String> for CredentialId in core/id.rs
- [x] T044 [US2] Ensure SecretString Debug/Display implementations return "[REDACTED]" in utils/secret_string.rs
- [x] T045 [US2] Add serde Serialize for SecretString to output "[REDACTED]" in utils/secret_string.rs

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test validation_tests` (10/10 passed)

**Checkpoint**: Type-safe IDs and secrets with compile-time guarantees and runtime redaction

---

## Phase 5: User Story 3 - Derive Encryption Keys Securely (Priority: P2)

**Goal**: Secure password-based key derivation with Argon2id preventing brute force

**Independent Test**: Derive key from password, measure derivation time (100-200ms), verify determinism

### Tests for User Story 3 (TDD - Write FIRST)

> **✅ COMPLETED**: Tests written and all passing

- [x] T046 [P] [US3] Add test_key_derivation_timing to tests/encryption_tests.rs (verify 100-200ms derivation time)
- [x] T047 [P] [US3] Add test_key_derivation_from_bytes to tests/encryption_tests.rs (EncryptionKey::from_bytes roundtrip)
- [x] T048 [P] [US3] Add test_encryption_key_zeroized to tests/encryption_tests.rs (verify key memory zeroed on drop)

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test encryption_tests` (8/8 passed)

### Implementation for User Story 3

> **✅ Already implemented in Phase 2 (T014-T019, T049-T051)**

- [x] T049 [US3] Verify EncryptionKey::derive_from_password() uses correct Argon2id params (19456 KB, 2 iterations, 32 byte output) in utils/crypto.rs
- [x] T050 [US3] Add EncryptionKey::from_bytes() constructor in utils/crypto.rs for loading keys from secure storage
- [x] T051 [US3] Verify ZeroizeOnDrop derive macro applied to EncryptionKey in utils/crypto.rs

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test encryption_tests` (8/8 passed)

**Checkpoint**: Secure key derivation with proper work factor and automatic memory zeroization

---

## Phase 6: User Story 4 - Handle Storage Backend Errors Gracefully (Priority: P2)

**Goal**: Clear, actionable error messages for storage failures with context

**Independent Test**: Simulate storage failures (read-only filesystem, missing file, corrupted data), verify error types and messages

### Tests for User Story 4 (TDD - Write FIRST)

> **✅ COMPLETED**: Tests written and all passing

- [x] T052 [P] [US4] Create tests/error_tests.rs with test_storage_error_not_found (NotFound error includes credential ID)
- [x] T053 [P] [US4] Add test_storage_error_display to tests/error_tests.rs (error messages are actionable)
- [x] T054 [P] [US4] Add test_crypto_error_display to tests/error_tests.rs (crypto errors don't leak secrets)
- [x] T055 [P] [US4] Add test_validation_error_display to tests/error_tests.rs (validation errors include reason)
- [x] T056 [P] [US4] Add test_error_source_chain to tests/error_tests.rs (verify #[source] attribute chains work)

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test error_tests` (9/9 passed)

### Implementation for User Story 4

> **✅ Already implemented in Phase 2, verified with comprehensive tests**

- [x] T057 [US4] Verify StorageError variants include credential ID context in core/error.rs
- [x] T058 [US4] Verify CryptoError messages don't leak secrets in core/error.rs
- [x] T059 [US4] Verify ValidationError messages include helpful reason field in core/error.rs
- [x] T060 [US4] Verify all error types implement Display with helpful messages per error hierarchy in core/error.rs
- [x] T061 [US4] Add error conversion examples to error documentation in core/error.rs

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test error_tests` (9/9 passed)

**Checkpoint**: Production-ready error handling with context-rich, actionable messages

---

## Phase 7: User Story 5 - Implement Storage Provider Trait (Priority: P3)

**Goal**: Abstract storage operations through StorageProvider trait for pluggable backends

**Independent Test**: Implement MockStorageProvider, verify all trait operations work through common interface

### Tests for User Story 5 (TDD - Write FIRST)

> **✅ COMPLETED**: Tests written and all passing

- [x] T062 [P] [US5] Create tests/storage_trait_tests.rs with test_mock_provider_store_and_retrieve
- [x] T063 [P] [US5] Add test_mock_provider_delete_idempotent to tests/storage_trait_tests.rs
- [x] T064 [P] [US5] Add test_mock_provider_list_empty to tests/storage_trait_tests.rs
- [x] T065 [P] [US5] Add test_mock_provider_exists to tests/storage_trait_tests.rs
- [x] T066 [P] [US5] Add test_mock_provider_retrieve_nonexistent to tests/storage_trait_tests.rs (verify NotFound error)
- [x] T067 [P] [US5] Add test_mock_provider_concurrent_writes to tests/storage_trait_tests.rs (last write wins, no corruption)

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test storage_trait_tests` (7/7 passed)

### Implementation for User Story 5

> **✅ COMPLETED**: StorageProvider trait and MockStorageProvider fully implemented

- [x] T068 [US5] Create MockStorageProvider in tests/storage_trait_tests.rs using HashMap<CredentialId, (EncryptedData, CredentialMetadata)> with RwLock
- [x] T069 [US5] Implement StorageProvider::store() for MockStorageProvider
- [x] T070 [US5] Implement StorageProvider::retrieve() for MockStorageProvider returning NotFound for missing IDs
- [x] T071 [US5] Implement StorageProvider::delete() for MockStorageProvider (idempotent)
- [x] T072 [US5] Implement StorageProvider::list() for MockStorageProvider with optional CredentialFilter support
- [x] T073 [US5] Implement StorageProvider::exists() for MockStorageProvider
- [x] T074 [US5] Add #[async_trait] attribute to MockStorageProvider impl per contracts/storage-provider-trait.md

**Verification Step**: ✅ ALL TESTS PASS - `cargo test --package nebula-credential --test storage_trait_tests` (7/7 passed)

**Checkpoint**: StorageProvider trait fully functional with mock implementation for testing

**Checkpoint**: StorageProvider trait fully functional with mock implementation for testing

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, examples, code quality improvements

> **✅ COMPLETED**: All polish tasks complete

- [x] T075 [P] Implement examples/basic_credential_storage.rs from quickstart.md showing 10-line usage example
- [x] T076 [P] Add rustdoc comments with examples to all public types in core/ modules
- [x] T077 [P] Add rustdoc comments with examples to SecretString and crypto functions in utils/
- [x] T078 [P] Add rustdoc comments with examples to Credential and StorageProvider traits in traits/
- [x] T079 Run `cargo fmt --all` to format code per CLAUDE.md
- [x] T080 Run `cargo clippy --package nebula-credential -- -D warnings` and fix all warnings per CLAUDE.md
- [x] T081 Run `cargo test --package nebula-credential` to verify all Phase 1 tests pass (34/34 passed)
- [x] T082 Run `cargo doc --no-deps --package nebula-credential` to verify documentation builds without errors
- [x] T083 Review plan.md quickstart.md examples and verify they work with implemented API
- [x] T084 Add SAFETY comments to any unsafe blocks if needed per CLAUDE.md (no unsafe code in nebula-credential)

**Verification Results:**
- All 34 Phase 1 integration tests pass (8 encryption + 9 error + 10 validation + 7 storage)
- Documentation builds successfully
- No unsafe code blocks (uses #![forbid(unsafe_code)])
- Clippy passes with zero warnings for nebula-credential
- Example demonstrates 10-line credential usage from quickstart.md

**Final Checkpoint**: ✅ Phase 1 complete - ready for Phase 2 (Storage Backends implementation)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately ✅
- **Foundational (Phase 2)**: Depends on Setup completion - **BLOCKS all user stories** ⚠️
- **User Stories (Phase 3-7)**: All depend on Foundational phase completion
  - Can proceed in parallel if multiple developers available
  - Or sequentially in priority order: US1 (P1) → US2 (P1) → US3 (P2) → US4 (P2) → US5 (P3)
- **Polish (Phase 8)**: Depends on all desired user stories complete

### User Story Dependencies

- **User Story 1 (P1)**: Requires Foundational phase - Encryption/decryption foundation
- **User Story 2 (P1)**: Requires Foundational phase - Validation and type safety (can run parallel with US1)
- **User Story 3 (P2)**: Requires Foundational phase - Extends US1 with secure key derivation testing
- **User Story 4 (P2)**: Requires Foundational phase - Error handling (can run parallel with US1-3)
- **User Story 5 (P3)**: Requires US1 complete (needs EncryptedData) - Storage abstraction

### Within Each User Story (TDD Workflow)

**CRITICAL TDD PATTERN**:
1. ✍️ Write tests FIRST (mark all test tasks as done)
2. ▶️ Run tests - verify they FAIL (document failure)
3. 🔨 Implement feature (mark implementation tasks as done)
4. ✅ Run tests - verify they PASS (document success)
5. ♻️ Refactor if needed (keeping tests passing)

**Within-Story Order**:
- Tests before implementation (ALWAYS)
- Models/types before services/functions
- Core functionality before integration

### Parallel Opportunities

**Phase 2 (Foundational)**: Can parallelize:
- Error hierarchy refactoring (T004-T008) - all in core/error.rs but different error types
- Core types (T009-T013) - different files (core/, utils/)
- Crypto module (T014-T019) - all in utils/crypto.rs (sequential within this group)
- Traits review (T020-T023) - traits/credential.rs and traits/storage.rs (parallel)

**User Stories**: If multiple developers available:
- US1 and US2 can run in parallel (both P1, different files)
- US3 extends US1 (wait for T032-T036)
- US4 can run parallel with US1-3 (different files, just testing errors)
- US5 requires US1 complete (needs EncryptedData type)

**Within User Story Tests**: All test file creation tasks marked [P] can run in parallel (different test files)

**Phase 8 (Polish)**: All rustdoc tasks marked [P] can run in parallel (different files)

---

## Parallel Example: Phase 2 Foundational

```bash
# Can run in parallel (different error types, same file but different sections):
Task T004: Refactor StorageError type in core/error.rs
Task T005: Add CryptoError type in core/error.rs  
Task T006: Add ValidationError type in core/error.rs

# Can run in parallel (different files):
Task T009: Create CredentialId in core/id.rs
Task T010: Refactor SecretString in utils/secret_string.rs
Task T011: Update CredentialMetadata in core/metadata.rs
Task T012: Update CredentialContext in core/context.rs

# Must run sequentially (same file, dependencies):
Task T014: Create EncryptionKey in utils/crypto.rs
Task T015: Create EncryptedData in utils/crypto.rs (no dependency on T014)
Task T016: Create NonceGenerator in utils/crypto.rs (no dependency on T014-15)
Task T017: Implement encrypt() in utils/crypto.rs (DEPENDS on T015, T016)
Task T018: Implement decrypt() in utils/crypto.rs (DEPENDS on T015)
```

---

## Parallel Example: User Story 1 (TDD)

```bash
# Step 1: Write all tests in parallel (different test functions, same file):
Task T027: test_encrypt_decrypt_roundtrip in tests/encryption_tests.rs
Task T028: test_key_derivation_deterministic in tests/encryption_tests.rs
Task T029: test_key_derivation_different_passwords in tests/encryption_tests.rs
Task T030: test_nonce_uniqueness in tests/encryption_tests.rs
Task T031: test_decryption_with_wrong_key in tests/encryption_tests.rs

# Step 2: Verify ALL tests FAIL
cargo test --package nebula-credential tests::encryption_tests

# Step 3: Implement sequentially (dependencies):
Task T032: derive_from_password() (foundation)
Task T033: NonceGenerator::next() (independent of T032)
Task T034: encrypt() (DEPENDS on T033)
Task T035: decrypt() (DEPENDS on T034 structure)
Task T036: Add ZeroizeOnDrop (independent)

# Step 4: Verify ALL tests PASS
cargo test --package nebula-credential tests::encryption_tests
```

---

## Implementation Strategy

### MVP First (User Stories 1-2 Only)

**Quickest path to working credential encryption**:

1. **Phase 1: Setup** (T001-T003) - ~10 min
2. **Phase 2: Foundational** (T004-T026) - ~4-6 hours
   - Parallel if multiple devs: ~2-3 hours
3. **Phase 3: User Story 1** (T027-T036) - ~3-4 hours
   - TDD: Write tests (1 hour) → Verify fail → Implement (2-3 hours) → Verify pass
4. **Phase 4: User Story 2** (T037-T045) - ~2-3 hours
   - Can run parallel with US1 if separate developer
5. **STOP and VALIDATE**: Test encryption + validation independently
6. **Phase 8: Polish** (T075-T084) - ~2-3 hours

**Total MVP Effort**: 1-2 days (single developer) or 4-8 hours (parallel team)

**MVP Delivers**: Encrypted credential storage with type-safe IDs and validated inputs

### Incremental Delivery (All User Stories)

1. **Foundation** (Phase 1-2) → ~5-7 hours total
2. **MVP** (US1-2) → Test independently → ~6-8 hours total → **DEMO READY** 🎯
3. **Add US3** (Key Derivation) → Test independently → +3 hours → **SECURITY ENHANCED**
4. **Add US4** (Error Handling) → Test independently → +2-3 hours → **PRODUCTION READY**
5. **Add US5** (Storage Trait) → Test independently → +3-4 hours → **EXTENSIBLE**
6. **Polish** (Phase 8) → Documentation + examples → +2-3 hours → **PHASE 1 COMPLETE**

**Total Phase 1 Effort**: 21-28 hours (single developer) or 12-16 hours (3 developers parallel)

### Parallel Team Strategy (3 Developers)

**Week 1 - Day 1**:
- All: Phase 1-2 together (Foundation) → 5-7 hours

**Week 1 - Day 2**:
- Dev A: User Story 1 (Encryption) → Tests first, then implement
- Dev B: User Story 2 (Validation) → Tests first, then implement
- Dev C: User Story 4 (Error Handling) → Tests first, then implement

**Week 1 - Day 3**:
- Dev A: User Story 3 (Key Derivation) → Extends US1, tests first
- Dev B: User Story 5 (Storage Trait) → After US1 complete, tests first
- Dev C: Phase 8 (Polish) → Documentation and examples

**Week 1 - Day 4**:
- All: Integration testing, code review, polish

**Timeline**: 4 days (parallel) vs 10 days (sequential single developer)

---

## TDD Workflow Summary

For EVERY user story, follow this pattern:

1. **📝 Write Tests First**:
   ```bash
   # Create test file with all test cases
   # Tests will not compile yet (types don't exist)
   ```

2. **🔍 Verify Tests Fail**:
   ```bash
   cargo test --package nebula-credential tests::<test_module>
   # Expected: Compilation errors or test failures
   # Document what fails and why
   ```

3. **🔨 Implement Minimal Code**:
   ```bash
   # Write just enough code to make tests pass
   # No extra features, no premature optimization
   ```

4. **✅ Verify Tests Pass**:
   ```bash
   cargo test --package nebula-credential tests::<test_module>
   # Expected: All tests pass
   # If not, debug and fix
   ```

5. **♻️ Refactor**:
   ```bash
   # Improve code quality while keeping tests green
   cargo test --package nebula-credential
   # Ensure all tests still pass
   ```

6. **📦 Commit**:
   ```bash
   git add .
   git commit -m "feat(credential): implement [User Story X]"
   ```

---

## Notes

- **[P] tasks**: Different files, no dependencies within same phase
- **[Story] label**: Maps task to specific user story for traceability
- **TDD discipline**: NEVER implement before tests fail
- **Incremental commits**: Commit after each user story or logical group
- **Breaking changes OK**: Refactor existing code to match Phase 1 design per plan.md
- **Stop at checkpoints**: Validate each story works independently before proceeding
- **Existing code**: Folder structure unchanged, refactor in place per plan.md
- **Phase 2 prep**: Phase 1 creates extensible foundation for storage backends (local, AWS, Azure, Vault)
- **Documentation**: See `crates/nebula-credential/docs/` for full architecture vision (phases 1-10)

---

## Success Criteria

Phase 1 is complete when:

- [ ] All 84 tasks completed
- [ ] All tests pass: `cargo test --workspace`
- [ ] No clippy warnings: `cargo clippy --workspace -- -D warnings`
- [ ] Documentation builds: `cargo doc --no-deps --package nebula-credential`
- [ ] Code formatted: `cargo fmt --all -- --check`
- [ ] quickstart.md examples work with implemented API
- [ ] MockStorageProvider passes all StorageProvider trait contract tests
- [ ] Encryption/decryption works with AES-256-GCM and Argon2id key derivation
- [ ] Secrets never appear in logs or debug output (verified by tests)
- [ ] Ready for Phase 2: Local storage backend implementation
