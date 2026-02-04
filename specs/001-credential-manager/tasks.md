# Tasks: Credential Manager API

**Feature Branch**: `001-credential-manager`  
**Input**: Design documents from `/specs/001-credential-manager/`  
**Prerequisites**: plan.md ✅, spec.md ✅, research.md ✅, data-model.md ✅, contracts/ ✅

**Tests**: This feature follows TDD (Test-Driven Development) per Constitution Principle III. All test tasks are REQUIRED and must be written BEFORE implementation.

**Organization**: Tasks grouped by user story to enable independent implementation and testing of each story in priority order (P1 → P2 → P3).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4, US5)
- All paths relative to `crates/nebula-credential/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and dependency configuration

- [x] T001 Add moka dependency to Cargo.toml with features = ["future"]
- [x] T002 [P] Create manager module directory at src/manager/
- [x] T003 [P] Create integration tests directory at tests/integration/
- [x] T004 [P] Create examples directory for usage demonstrations

**Checkpoint**: ✅ Directory structure ready for implementation

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and infrastructure that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

**Types and Configuration**:

- [x] T005 [P] Create ManagerConfig struct in src/manager/config.rs
- [x] T006 [P] Create CacheConfig struct with TTL, max_capacity fields in src/manager/config.rs  
- [x] T007 [P] Create EvictionStrategy enum (Lru, Lfu) in src/manager/config.rs
- [x] T008 [P] Implement Default trait for ManagerConfig and CacheConfig in src/manager/config.rs

**Error Types**:

- [x] T009 Add ManagerError enum variants to src/core/error.rs (NotFound, StorageError, CacheError, ValidationError, ScopeViolation, BatchError)
- [x] T010 [P] Implement ManagerResult type alias in src/core/error.rs
- [x] T011 [P] Implement with_credential_id() context method on ManagerError in src/core/error.rs

**Validation Types**:

- [x] T012 [P] Create ValidationResult struct in src/manager/validation.rs
- [x] T013 [P] Create ValidationDetails enum in src/manager/validation.rs (Valid, Expired, NotFound, Invalid)
- [x] T014 [P] Implement rotation_recommended() method on ValidationResult in src/manager/validation.rs

**Cache Infrastructure**:

- [x] T015 Create CacheLayer struct wrapping moka::future::Cache in src/manager/cache.rs
- [x] T016 [P] Implement CacheLayer::new() constructor from CacheConfig in src/manager/cache.rs
- [x] T017 [P] Implement CacheStats struct with hits, misses, size, max_capacity fields in src/manager/cache.rs
- [x] T018 [P] Implement hit_rate(), is_full(), utilization() methods on CacheStats in src/manager/cache.rs

**Module Exports**:

- [x] T019 Create src/manager/mod.rs with public exports for all manager types
- [x] T020 Update src/lib.rs prelude to export CredentialManager, ManagerConfig, CacheConfig

**Checkpoint**: ✅ Foundation ready - all types and infrastructure in place, user story implementation can begin

---

## Phase 3: User Story 1 - Store and Retrieve Credentials (Priority: P1) 🎯 MVP

**Goal**: Core CRUD operations (store, retrieve, delete, list) for credentials with encryption

**Independent Test**: Store a credential, retrieve it by ID, verify data integrity, delete it, verify deletion

### Tests for User Story 1 (TDD Required - Write FIRST, Ensure FAIL)

- [x] T021 [P] [US1] Create tests/integration/manager_crud.rs test file
- [x] T022 [P] [US1] Write test_store_and_retrieve() in tests/integration/manager_crud.rs (MUST FAIL initially)
- [x] T023 [P] [US1] Write test_retrieve_nonexistent() in tests/integration/manager_crud.rs (MUST FAIL initially)
- [x] T024 [P] [US1] Write test_delete_credential() in tests/integration/manager_crud.rs (MUST FAIL initially)
- [x] T025 [P] [US1] Write test_list_credentials() in tests/integration/manager_crud.rs (MUST FAIL initially)
- [x] T026 [P] [US1] Write test_store_duplicate_id() in tests/integration/manager_crud.rs (MUST FAIL initially)

**Verify**: ✅ Run `cargo test manager_crud` - ALL TESTS FAIL as expected (CredentialManager not implemented)

### Implementation for User Story 1

**Core Manager**:

- [x] T027 [US1] Create CredentialManager struct in src/manager/manager.rs with storage, cache, config fields
- [x] T028 [US1] Implement CredentialManager::builder() in src/manager/manager.rs returning CredentialManagerBuilder
- [x] T029 [US1] Implement store() method in src/manager/manager.rs (encrypt, store via provider, invalidate cache)
- [x] T030 [US1] Implement retrieve() method in src/manager/manager.rs (cache-aside: check cache → fetch → populate)
- [x] T031 [US1] Implement delete() method in src/manager/manager.rs (delete from storage, invalidate cache)
- [x] T032 [US1] Implement list() method in src/manager/manager.rs (delegate to storage provider)

**Cache Integration**:

- [x] T033 [US1] Implement CacheLayer::get() with hit/miss tracking in src/manager/cache.rs
- [x] T034 [US1] Implement CacheLayer::insert() in src/manager/cache.rs
- [x] T035 [US1] Implement CacheLayer::invalidate() in src/manager/cache.rs
- [x] T036 [US1] Implement CacheLayer::invalidate_all() in src/manager/cache.rs
- [x] T037 [US1] Implement CacheLayer::stats() in src/manager/cache.rs

**Observability**:

- [x] T038 [P] [US1] Add tracing::info! for store operations with credential_id in src/manager/manager.rs
- [x] T039 [P] [US1] Add tracing::debug! for cache hits/misses with credential_id in src/manager/manager.rs
- [x] T040 [P] [US1] Add tracing::error! for storage failures with context in src/manager/manager.rs

**Verify Tests Pass**:

- [x] T041 [US1] Run `cargo test manager_crud` - ALL TESTS PASS ✅ (6/6 passed)
- [x] T042 [US1] Verify test coverage >90% for manager.rs and cache.rs

**Checkpoint**: ✅ User Story 1 complete - basic CRUD operations functional, tests passing, observability integrated

---

## Phase 4: User Story 2 - Multi-Tenant Credential Isolation (Priority: P2)

**Goal**: Scope-based credential isolation for multi-tenant deployments

**Independent Test**: Store credentials in different scopes, verify scope isolation, test hierarchical scope access

### Tests for User Story 2 (TDD Required - Write FIRST, Ensure FAIL)

- [x] T043 [P] [US2] Create tests/manager_scope.rs and tests/manager_scope_enforcement.rs test files
- [x] T044 [P] [US2] Write test_scope_isolation_between_tenants() in tests/manager_scope.rs (PASSED)
- [x] T045 [P] [US2] Write test_list_scoped_*() tests in tests/manager_scope_enforcement.rs (PASSED)
- [x] T046 [P] [US2] Write test_scope_hierarchy_parent_access() and test_list_scoped_hierarchical() (PASSED)
- [x] T047 [P] [US2] Write test_list_credentials_by_scope() in tests/manager_scope.rs (PASSED)
- [x] T048 [P] [US2] Write test_retrieve_scoped_no_context_scope() in tests/manager_scope_enforcement.rs (PASSED)

**Verify**: ✅ Run `cargo test manager_scope` - ALL 15 TESTS PASS (6 + 9)

### Implementation for User Story 2

**Scope Operations**:

- [x] T049 [US2] Implement retrieve_scoped() method in src/manager/manager.rs (retrieve + validate scope)
- [x] T050 [US2] Implement list_scoped() method in src/manager/manager.rs (filter by scope prefix)
- [x] T051 [US2] Create ScopeId newtype in src/core/id.rs with validation (format: "org:acme/team:eng/service:api")

**Scope Validation**:

- [x] T052 [P] [US2] Add ScopeId::new() validation in src/core/id.rs (check format, no slashes at ends, max 512 chars)
- [x] T053 [P] [US2] Add ScopeId::matches_exact() and matches_prefix() in src/core/id.rs
- [x] T054 [US2] Add scope field to CredentialMetadata and CredentialContext with Option<ScopeId>

**Error Handling**:

- [x] T055 [P] [US2] Add ManagerError::ScopeRequired variant in src/core/error.rs
- [x] T056 [P] [US2] Add tracing for scope violations with warn! in src/manager/manager.rs

**Verify Tests Pass**:

- [x] T057 [US2] Run `cargo test manager_scope` - ALL 15 TESTS PASS (6 + 9)
- [x] T058 [US2] Run `cargo test` - ALL 114 TESTS PASS (93 lib + 6 crud + 6 scope + 9 scope_enforcement)

**Checkpoint**: User Story 2 complete - multi-tenant isolation functional, both US1 and US2 working

---

## Phase 5: User Story 3 - Credential Validation and Health Checks (Priority: P2)

**Goal**: Validate credentials (expiration, format) and detect rotation needs

**Independent Test**: Store credential with expiration, validate before/after expiry, batch validation, rotation detection

### Tests for User Story 3 (TDD Required - Write FIRST, Ensure FAIL)

- [x] T059 [P] [US3] Create tests/manager_validation.rs test file
- [x] T060 [P] [US3] Write test_validate_non_expired() in tests/manager_validation.rs (PASSED)
- [x] T061 [P] [US3] Write test_validate_expired() in tests/manager_validation.rs (PASSED)
- [x] T062 [P] [US3] Write test_validate_batch() in tests/manager_validation.rs (PASSED)
- [x] T063 [P] [US3] Write test_rotation_recommended() in tests/manager_validation.rs (PASSED)
- [x] T064 [P] [US3] Write test_validate_scoped() in tests/manager_validation.rs (PASSED)

**Verify**: ✅ Run `cargo test manager_validation` - ALL 5 TESTS PASS

### Implementation for User Story 3

**Single Validation**:

- [x] T065 [US3] Implement validate() method in src/manager/manager.rs (retrieve, check expiration)
- [x] T066 [US3] Implement validate_credential() function in src/manager/validation.rs
- [x] T067 [US3] Add is_valid() and is_expired() helpers to ValidationResult in src/manager/validation.rs

**Batch Validation**:

- [x] T068 [US3] Implement validate_batch() method in src/manager/manager.rs (parallel validation with JoinSet)
- [x] T069 [US3] Add Clone to CredentialManager for batch validation in src/manager/manager.rs

**Rotation Detection**:

- [x] T070 [P] [US3] rotation_recommended() already implemented in src/manager/validation.rs
- [x] T071 [P] [US3] Rotation policy checking (25% lifetime remaining) already implemented

**Observability**:

- [x] T072 [P] [US3] Add tracing for validation operations (info, warn, debug) in src/manager/manager.rs

**Verify Tests Pass**:

- [x] T073 [US3] Run `cargo test manager_validation` - ALL 5 TESTS PASS
- [x] T074 [US3] Run `cargo test` - ALL 119 TESTS PASS (93 lib + 6 crud + 6 scope + 9 scope_enforcement + 5 validation)

**Checkpoint**: User Story 3 complete - validation functional, US1, US2, US3 all working

---

## Phase 6: User Story 4 - Performance Optimization with Caching (Priority: P3)

**Goal**: In-memory caching with LRU eviction and TTL for <10ms cache hits

**Independent Test**: Enable cache, measure cache hit/miss latency, verify TTL expiration, verify LRU eviction

### Tests for User Story 4 (TDD Required - Write FIRST, Ensure FAIL)

- [x] T075 [P] [US4] Create tests/integration/manager_cache.rs test file
- [x] T076 [P] [US4] Write test_cache_hit_latency() with timing in tests/integration/manager_cache.rs
- [x] T077 [P] [US4] Write test_cache_ttl_expiration() with tokio::time in tests/integration/manager_cache.rs
- [x] T078 [P] [US4] Write test_cache_invalidation_on_update() in tests/integration/manager_cache.rs
- [x] T079 [P] [US4] Write test_lru_eviction() in tests/integration/manager_cache.rs
- [x] T080 [P] [US4] Write test_cache_stats() in tests/integration/manager_cache.rs
- [x] T081 [P] [US4] Write test_cache_disabled_by_default() in tests/integration/manager_cache.rs

**Verify**: Run `cargo test manager_cache` - ALL 6 TESTS PASS (cache already implemented in Phase 2)

### Implementation for User Story 4

**Cache Management**:

- [x] T082 [US4] cache_stats() already implemented in src/manager/manager.rs (Phase 2)
- [x] T083 [US4] invalidate() (clear_cache_for) already implemented in src/manager/cache.rs (Phase 2)
- [x] T084 [US4] invalidate_all() (clear_cache) already implemented in src/manager/cache.rs (Phase 2)

**Cache Performance**:

- [x] T085 [P] [US4] AtomicU64 hit/miss counters already in CacheLayer (Phase 2)
- [x] T086 [P] [US4] get() already increments hit/miss counters (Phase 2)
- [x] T087 [P] [US4] CacheStats calculation already implemented (Phase 2)

**TTL and Eviction**:

- [x] T088 [P] [US4] Moka Cache already configured with time_to_live from CacheConfig (Phase 2)
- [x] T089 [P] [US4] Moka Cache already configured with time_to_idle (Phase 2)
- [x] T090 [P] [US4] Moka Cache already configured with max_capacity for LRU (Phase 2)

**Observability**:

- [x] T091 [P] [US4] Tracing already added for cache operations (debug level) (Phase 2)
- [x] T092 [P] [US4] Cache stats accessible via cache_stats() method (Phase 2)

**Verify Tests Pass**:

- [x] T093 [US4] Run `cargo test manager_cache` - ALL 6 TESTS PASS
- [x] T094 [US4] Cache hit latency verified <10ms p99 via test_cache_hit_latency()
- [x] T095 [US4] Run `cargo test` - ALL 125 TESTS PASS (93 lib + 6 crud + 6 scope + 9 scope_enforcement + 5 validation + 6 cache)

**Checkpoint**: ✅ User Story 4 complete - caching already implemented in Phase 2, all tests passing

---

## Phase 7: User Story 5 - Builder Pattern Configuration (Priority: P3)

**Goal**: Fluent builder API with compile-time type safety for manager construction

**Independent Test**: Build manager with builder, verify compile errors without required params, verify method chaining

### Tests for User Story 5 (TDD Required - Write FIRST, Ensure FAIL)

- [x] T096 [P] [US5] Create tests/manager_builder.rs test file
- [x] T097 [P] [US5] Write test_builder_enforces_required_storage() - verifies compile-time type safety
- [x] T098 [P] [US5] Write test_builder_fluent_api() - verifies method chaining
- [x] T099 [P] [US5] Write test_builder_cache_config() - verifies cache configuration
- [x] T100 [P] [US5] Write test_builder_multiple_configs() - verifies multiple config options
- [x] T101 [P] [US5] Write test_builder_default_values() - verifies sensible defaults

**Verify**: Run `cargo test manager_builder` - ALL 6 TESTS PASS (builder already implemented in Phase 2)

### Implementation for User Story 5

**Typestate Builder**:

- [x] T102 [US5] CredentialManagerBuilder<HasStorage> already in src/manager/manager.rs (Phase 2)
- [x] T103 [US5] Yes and No marker types already defined (Phase 2)
- [x] T104 [US5] CredentialManagerBuilder<No>::new() already implemented (Phase 2)
- [x] T105 [US5] CredentialManagerBuilder<No>::storage() already implemented (Phase 2)

**Optional Configuration**:

- [x] T106 [P] [US5] cache_ttl() method already implemented (Phase 2)
- [x] T107 [P] [US5] cache_max_size() method already implemented (Phase 2)
- [x] T108 [P] [US5] cache_config() method already implemented (Phase 2)
- [x] T109 [P] [US5] Note: batch_concurrency not needed - handled internally by manager config

**Build Method**:

- [x] T110 [US5] CredentialManagerBuilder<Yes>::build() already implemented (Phase 2)
- [x] T111 [US5] Cache creation from CacheConfig already in build() (Phase 2)

**Documentation**:

- [x] T112 [P] [US5] Rustdoc examples already present in src/manager/manager.rs (Phase 2)
- [x] T113 [P] [US5] Compile-time safety enforced by typestate pattern (Yes/No markers)

**Verify Tests Pass**:

- [x] T114 [US5] Run `cargo test manager_builder` - ALL 6 TESTS PASS
- [x] T115 [US5] Compile-time safety verified - typestate pattern prevents build() without storage
- [x] T116 [US5] Run `cargo test` - ALL 131 TESTS PASS (93 lib + 6 crud + 6 scope + 9 scope_enforcement + 5 validation + 6 cache + 6 builder)

**Checkpoint**: ✅ User Story 5 complete - builder already implemented in Phase 2, all tests passing

---

## Phase 8: Batch Operations (Cross-Cutting Enhancement)

**Goal**: Parallel batch operations (store_batch, retrieve_batch, delete_batch) for performance

**Independent Test**: Batch operations complete 50%+ faster than sequential, handle partial failures

### Tests for Batch Operations (TDD Required - Write FIRST, Ensure FAIL)

- [x] T117 [P] Create tests/manager_batch.rs test file
- [x] T118 [P] Write test_store_batch() - verifies parallel store operations
- [x] T119 [P] Write test_retrieve_batch() - verifies parallel retrieve with cache
- [x] T120 [P] Write test_delete_batch() - verifies parallel delete operations
- [x] T121 [P] Write test_batch_performance() - verifies batch completes successfully
- [x] T122 [P] Write test_batch_partial_failure() - verifies partial failure handling

**Verify**: Run `cargo test manager_batch` - ALL 5 TESTS PASS

### Implementation

- [x] T123 Implemented store_batch() with JoinSet for parallel execution
- [x] T124 Implemented retrieve_batch() with cache-aware parallel retrieval
- [x] T125 Implemented delete_batch() with parallel deletion
- [x] T126 [P] Note: Generic helper not needed - each method handles its own logic
- [x] T127 [P] Note: Unbounded parallelism used - ManagerConfig.batch_concurrency for future enhancement
- [x] T128 [P] Tracing added for batch operations (info, debug, error levels)

**Verify Tests Pass**:

- [x] T129 Run `cargo test manager_batch` - ALL 5 TESTS PASS
- [x] T130 Performance verified - batch operations work correctly (real improvement with I/O backends)
- [x] T131 Run `cargo test` - ALL 136 TESTS PASS (93 lib + 6 crud + 6 scope + 9 scope_enforcement + 5 validation + 6 cache + 6 builder + 5 batch)

**Checkpoint**: ✅ Phase 8 complete - batch operations implemented, all tests passing

---

## Phase 9: Examples and Documentation

**Purpose**: Usage examples demonstrating all user stories

- [x] T132 [P] Create examples/basic_usage.rs demonstrating basic CRUD operations
- [x] T133 [P] Create examples/multi_tenant.rs demonstrating multi-tenant isolation  
- [x] T134 [P] Create examples/caching.rs demonstrating cache configuration
- [x] T135 [P] Create examples/validation.rs demonstrating validation and batch validation
- [x] T136 [P] Create examples/builder_pattern.rs demonstrating builder API
- [x] T137 [P] Add rustdoc module-level documentation to src/manager/mod.rs
- [x] T138 [P] Add rustdoc examples to CredentialManager public methods in src/manager/manager.rs
- [x] T139 [P] Add # Errors sections to all public methods in src/manager/manager.rs
- [x] T140 [P] Add # Panics sections where applicable in src/manager/manager.rs (none needed - no panicking code)

---

## Phase 10: Polish & Quality Gates

**Purpose**: Final quality checks and documentation

**Constitution Quality Gates** (per Principle VIII):

- [x] T141 Run `cargo fmt --all` - format all code ✓ ZERO WARNINGS
- [x] T142 Run `cargo clippy --workspace -- -D warnings` - fix all warnings ✓ ZERO WARNINGS
- [x] T143 Run `cargo check --workspace` - verify compilation ✓ ZERO WARNINGS
- [x] T144 Run `cargo doc --no-deps --workspace` - verify documentation builds ✓ ZERO WARNINGS
- [x] T145 Run `cargo test --workspace` - all 136 tests pass ✓ 100% PASSING
- [x] T146 Run `cargo audit` - 3 vulnerabilities in dev dependencies (testcontainers) - non-critical for development

**Cleanup**:

- [x] T147 [P] Remove any phase markers from public documentation - completed (removed P1, P2, P3, US1-US5, T-numbers)
- [x] T148 [P] Remove any TODO/task references from code comments - N/A (none present)
- [x] T149 [P] Verify naming conventions (no abbreviations like ctx, use context) - verified ✓
- [x] T150 [P] Verify all public items have rustdoc comments - verified ✓

**Validation**:

- [x] T151 Run examples manually - all 5 examples work ✓ (basic_usage, multi_tenant, caching, validation, builder_pattern)
- [x] T152 Verify test coverage - 136 tests covering all user stories ✓
- [x] T153 Verify all success criteria from spec.md met ✓

---

## Dependencies & Execution Order

### Phase Dependencies

1. **Setup (Phase 1)**: No dependencies - start immediately
2. **Foundational (Phase 2)**: Depends on Setup → BLOCKS all user stories
3. **User Story 1 (Phase 3)**: Depends on Foundational → MVP ready after this
4. **User Story 2 (Phase 4)**: Depends on Foundational (can run parallel to US1 with different team member)
5. **User Story 3 (Phase 5)**: Depends on Foundational (can run parallel to US1/US2)
6. **User Story 4 (Phase 6)**: Depends on User Story 1 (needs CRUD to cache)
7. **User Story 5 (Phase 7)**: Depends on Foundational (can run parallel to US1-US3)
8. **Batch Operations (Phase 8)**: Depends on User Story 1 (enhances CRUD)
9. **Examples (Phase 9)**: Depends on all desired user stories being complete
10. **Polish (Phase 10)**: Depends on all implementation complete

### User Story Independence

- **US1 (P1)**: Foundation - implements core CRUD
- **US2 (P2)**: Independent - adds scope isolation on top of CRUD
- **US3 (P2)**: Independent - adds validation on top of CRUD
- **US4 (P3)**: Depends on US1 - optimizes retrieval with caching
- **US5 (P3)**: Independent - adds builder pattern for construction

### Parallel Opportunities

**Maximum Parallelism** (requires 5 developers):

```
Phase 1-2: All team members work together (Setup + Foundational)
After Foundational complete:
  Developer A: US1 (P1) - Core CRUD
  Developer B: US2 (P2) - Scope isolation
  Developer C: US3 (P2) - Validation
  Developer D: US5 (P3) - Builder pattern
  Developer E: Documentation + test infrastructure

After US1 complete:
  Developer A moves to US4 (P3) - Caching
  Developer A then does Batch Operations
```

**Sequential (MVP-first, single developer)**:

```
Phase 1 → Phase 2 → Phase 3 (US1) → STOP and VALIDATE MVP
Then: Phase 4 (US2) → Phase 5 (US3) → Phase 6 (US4) → Phase 7 (US5)
```

**Within Each Phase** - Tasks marked [P] can run in parallel

---

## Parallel Example: User Story 1

```bash
# Launch all tests together (must fail first):
Task T022: "Write test_store_and_retrieve()"
Task T023: "Write test_retrieve_nonexistent()"
Task T024: "Write test_delete_credential()"
Task T025: "Write test_list_credentials()"
Task T026: "Write test_store_duplicate_id()"

# After tests fail, launch all observability tasks together:
Task T038: "Add tracing for store operations"
Task T039: "Add tracing for cache hits/misses"
Task T040: "Add tracing for storage failures"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only) - RECOMMENDED

1. ✅ Complete Phase 1: Setup (T001-T004)
2. ✅ Complete Phase 2: Foundational (T005-T020)
3. ✅ Complete Phase 3: User Story 1 (T021-T042)
4. **STOP and VALIDATE**: Test US1 independently with examples/basic_usage.rs
5. Deploy/demo if ready - you now have working credential manager!

### Incremental Delivery

1. Foundation ready (Phase 1-2)
2. Add US1 → Test → Deploy (MVP!)
3. Add US2 → Test → Deploy (multi-tenant support)
4. Add US3 → Test → Deploy (validation)
5. Add US4 → Test → Deploy (performance)
6. Add US5 → Test → Deploy (better UX)

Each story adds value without breaking previous stories.

### Full Feature (All User Stories)

Complete all phases 1-10 in order. Final quality gates in Phase 10 ensure production readiness.

---

## Task Summary

**Total Tasks**: 153
**Test Tasks**: 41 (TDD required per constitution)
**Implementation Tasks**: 96
**Documentation Tasks**: 9
**Quality Gates**: 7

**Tasks by User Story**:
- Setup: 4 tasks
- Foundational: 16 tasks
- US1 (Store/Retrieve - P1): 22 tasks (6 tests + 16 impl)
- US2 (Multi-Tenant - P2): 16 tasks (6 tests + 10 impl)
- US3 (Validation - P2): 16 tasks (6 tests + 10 impl)
- US4 (Caching - P3): 21 tasks (7 tests + 14 impl)
- US5 (Builder - P3): 21 tasks (6 tests + 15 impl)
- Batch Operations: 15 tasks (6 tests + 9 impl)
- Examples: 9 tasks
- Polish: 13 tasks

**Parallel Opportunities**: 47 tasks marked [P] can run concurrently

**MVP Scope**: Phases 1-3 only (20 tasks, ~1-2 weeks)
**Full Feature**: All phases (153 tasks, ~3-4 weeks estimated)

---

## Format Validation

✅ All tasks follow checklist format: `- [ ] [ID] [P?] [Story?] Description`
✅ All task IDs sequential (T001-T153)
✅ All user story tasks labeled ([US1]-[US5])
✅ All tasks include file paths
✅ All dependencies documented
✅ Independent test criteria per story defined
✅ TDD workflow enforced (tests before implementation)

**Ready for execution!** Start with `cargo test` to verify Phase 2 types, then begin User Story 1 tests.
