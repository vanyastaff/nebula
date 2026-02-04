# Implementation Plan: Credential Manager API

**Branch**: `001-credential-manager` | **Date**: 2026-02-04 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-credential-manager/spec.md`

## Summary

Implement the high-level Credential Manager API that provides CRUD operations, caching, validation, and builder pattern configuration for managing credentials across multiple storage backends. This is Phase 3 of the nebula-credential roadmap, building on Phase 1 (Core Abstractions) and Phase 2 (Storage Backends) which are already implemented.

**Technical Approach**: Create a `CredentialManager` struct that wraps storage providers with an in-memory caching layer, exposes fluent builder API for configuration, and provides async CRUD operations with scope-based multi-tenant isolation. Use `moka` crate for LRU cache with TTL expiration, implement batch operations for performance, and ensure thread safety with Arc/RwLock patterns.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)

**Primary Dependencies**:
- **Tokio** (1.x) - Async runtime for non-blocking I/O
- **async-trait** (0.1) - Async trait methods for Credential trait
- **serde** (1.x) - Serialization/deserialization of credentials
- **thiserror** (1.x) - Error type definitions per constitution
- **moka** (0.12) - In-memory cache with LRU eviction and TTL
- **tracing** (0.1) - Structured logging and observability
- **zeroize** (1.8) - Zero-on-drop for SecretString security

**Storage**: Abstracted via `StorageProvider` trait (implemented in Phase 2):
- **LocalStorage** - File-based with atomic writes (default feature)
- **AWS Secrets Manager** - Cloud KMS integration (optional feature)
- **HashiCorp Vault** - Enterprise secrets management (optional feature)
- **Kubernetes Secrets** - Container-native storage (optional feature)

**Testing**: 
- `cargo test --workspace` for unit and integration tests
- `#[tokio::test(flavor = "multi_thread")]` for async tests
- `tokio::time::pause()` and `advance()` for time-based cache tests
- Mock storage provider already exists for testing without external dependencies

**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)

**Project Type**: Workspace crate (`crates/nebula-credential/`) - part of 16-crate Nebula workspace

**Performance Goals**:
- **Cache hits**: <10ms p99 latency (in-memory lookup)
- **Cache misses**: <100ms p99 latency (storage provider + decryption)
- **Batch operations**: 50% reduction vs sequential operations
- **Cache hit rate**: >80% under typical 5:1 read-write ratio
- **Throughput**: 10,000 concurrent operations without contention
- **Memory**: Bounded cache with LRU eviction (default 1000 entries)

**Constraints**:
- **No circular dependencies**: CredentialManager cannot depend on higher-layer crates
- **Isolated errors**: `CredentialError` defined in nebula-credential (no shared error crate)
- **Thread safety**: All operations must be Send + Sync for multi-threaded Tokio runtime
- **Zero-copy secrets**: SecretString must zeroize memory on drop
- **Scope isolation**: Multi-tenant boundaries enforced at manager level

**Scale/Scope**:
- **Credential count**: Designed for 1000s-10000s of credentials per deployment
- **Concurrent access**: 100s-1000s of simultaneous credential retrievals
- **Scope depth**: Support 3-4 level hierarchical scopes (org → team → service)
- **Cache size**: Default 1000 entries, configurable up to 100,000
- **Batch size**: Up to 100 credentials per batch operation

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Uses newtype patterns (CredentialId, ScopeId already implemented in Phase 1), enums for state, no stringly-typed data
- [x] **Isolated Error Handling**: `CredentialError` defined in `nebula-credential/src/core/error.rs` using thiserror, converts from StorageError/CacheError at boundaries
- [x] **Test-Driven Development**: Tests will be written before implementation (TDD required by constitution, plan includes test strategy)
- [x] **Async Discipline**: All manager operations are async with `#[async_trait]`, timeouts on storage operations (5s default), cancellation support via tokio::select
- [x] **Modular Architecture**: Stays within nebula-credential crate (Domain layer), no dependencies on Business/Presentation layers, only uses Core layer (nebula-core, nebula-log)
- [x] **Observability**: All CRUD operations emit tracing events with credential_id, scope_id context, cache hits/misses tracked, error logs with full context
- [x] **Simplicity**: No premature abstractions - single CredentialManager struct with builder, cache is optional (default disabled), complexity justified below

**Constitution Re-Check After Design** (Phase 1 complete):
- [x] Type safety maintained - All public APIs use strong types, builder prevents invalid configurations
- [x] Errors remain isolated - No new cross-crate error dependencies introduced
- [x] Async patterns correct - Proper use of Arc for shared state, RwLock for cache, no blocking operations
- [x] Observability complete - Tracing spans for all operations, metrics for cache performance
- [x] Simplicity preserved - Linear design without unnecessary abstraction layers

## Project Structure

### Documentation (this feature)

```text
specs/001-credential-manager/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output - caching library comparison
├── data-model.md        # Phase 1 output - CredentialManager, CacheConfig types
├── quickstart.md        # Phase 1 output - 3-minute getting started guide
├── contracts/           # Phase 1 output - API contracts
│   └── manager-api.md   # Manager API documentation
└── tasks.md             # Phase 2 output (/speckit.tasks - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
crates/nebula-credential/
├── src/
│   ├── lib.rs                    # [MODIFY] Add CredentialManager to prelude
│   ├── core/
│   │   ├── mod.rs                # [EXISTS] Re-exports core types
│   │   ├── id.rs                 # [EXISTS] CredentialId, ScopeId (Phase 1)
│   │   ├── context.rs            # [EXISTS] CredentialContext (Phase 1)
│   │   ├── metadata.rs           # [EXISTS] CredentialMetadata (Phase 1)
│   │   ├── error.rs              # [MODIFY] Add ManagerError variants
│   │   └── filter.rs             # [EXISTS] CredentialFilter (Phase 1)
│   ├── manager/                  # [NEW] Credential manager implementation
│   │   ├── mod.rs                # Public API exports
│   │   ├── manager.rs            # CredentialManager core struct
│   │   ├── builder.rs            # Builder pattern for configuration
│   │   ├── cache.rs              # Cache layer with moka integration
│   │   ├── config.rs             # ManagerConfig, CacheConfig types
│   │   └── validation.rs         # Batch validation logic
│   ├── traits/
│   │   ├── mod.rs                # [EXISTS] Trait re-exports
│   │   ├── storage.rs            # [EXISTS] StorageProvider trait (Phase 1)
│   │   └── credential.rs         # [EXISTS] Credential trait (Phase 1)
│   ├── providers/                # [EXISTS] Storage implementations (Phase 2)
│   │   ├── mod.rs                # [EXISTS]
│   │   ├── local.rs              # [EXISTS] LocalStorageProvider
│   │   ├── aws.rs                # [EXISTS] AWS Secrets Manager
│   │   ├── vault.rs              # [EXISTS] HashiCorp Vault
│   │   └── kubernetes.rs         # [EXISTS] K8s Secrets
│   └── utils/
│       ├── mod.rs                # [EXISTS] Utility re-exports
│       ├── secret_string.rs      # [EXISTS] SecretString (Phase 1)
│       ├── crypto.rs             # [EXISTS] Encryption utilities (Phase 1)
│       └── retry.rs              # [EXISTS] Retry logic (Phase 2)
├── tests/
│   ├── integration/              # [NEW] Integration tests
│   │   ├── manager_crud.rs       # CRUD operations tests
│   │   ├── manager_scopes.rs     # Multi-tenant scope isolation tests
│   │   ├── manager_cache.rs      # Caching behavior tests
│   │   └── manager_batch.rs      # Batch operations tests
│   └── common/
│       └── mod.rs                # [EXISTS] Test utilities
├── examples/
│   ├── basic_usage.rs            # [NEW] Simple CRUD example
│   ├── multi_tenant.rs           # [NEW] Scope isolation example
│   └── caching.rs                # [NEW] Cache configuration example
└── Cargo.toml                    # [MODIFY] Add moka dependency
```

**Structure Decision**: 
- **Affected Crate**: `nebula-credential` (Domain layer - credential management)
- **New Module**: `manager/` module within existing crate (not a new crate)
- **Justification**: Follows Constitution Principle V (Modular Architecture) by keeping credential management consolidated in one crate. CredentialManager is a core domain service that orchestrates storage providers and caching, fitting naturally in the Domain layer alongside Phase 1 traits and Phase 2 providers. No new crate needed as this is a logical extension of existing credential functionality.
- **Architectural Layer**: Domain layer per `docs/nebula-architecture-overview.md`
- **Dependencies**: Only depends on lower layers (Core: nebula-core, nebula-log; Infrastructure: tokio, moka)

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

No violations. All constitution principles satisfied:
- Type safety via builder pattern with compile-time checks
- Isolated errors in nebula-credential crate
- TDD enforced for all new manager functionality
- Async operations with proper timeouts and cancellation
- Stays within Domain layer architectural boundary
- Comprehensive observability with tracing
- Simple design with single manager struct + optional caching

## Phase 0: Research & Technology Decisions

**Status**: ✅ COMPLETED

### Research Areas

#### 1. Caching Library Selection

**Decision**: Use `moka` (v0.12) for in-memory caching

**Rationale**:
- **LRU Eviction**: Built-in least-recently-used eviction when cache reaches capacity
- **TTL Support**: Time-based expiration per entry or global TTL
- **Async-First**: Native `moka::future::Cache` for async/await patterns
- **Thread-Safe**: Arc-based internal design, safe for concurrent access
- **Performance**: Lock-free read paths for high-throughput scenarios
- **Metrics**: Built-in hit/miss counters and eviction tracking
- **Mature**: 2+ years in production, used by major Rust projects

**Alternatives Considered**:
- **mini-moka**: Lighter weight but lacks TTL support
- **cached**: Macro-based, harder to integrate with dynamic configuration
- **lru**: Low-level, requires manual TTL implementation
- **dashmap + custom TTL**: More complex, reinventing wheel

#### 2. Scope Hierarchy Implementation

**Decision**: Flat scope matching with prefix-based hierarchical queries

**Rationale**:
- **Simplicity**: Scope stored as string (e.g., "org:acme/team:eng/service:api")
- **Flexibility**: Can query by prefix for hierarchical access
- **Performance**: Simple string comparison, no tree traversal overhead
- **Storage**: Works with existing CredentialFilter from Phase 1

**Alternatives Considered**:
- **Tree Structure**: More complex, harder to persist in storage providers
- **Separate Scope Table**: Requires schema changes in all providers
- **Graph Database**: Massive overkill for 3-4 level hierarchy

#### 3. Builder Pattern Type Safety

**Decision**: Use typestate pattern with PhantomData for compile-time validation

**Rationale**:
- **Compile-Time Safety**: Cannot call `.build()` without required fields
- **Ergonomic**: Fluent API with method chaining
- **Standard Pattern**: Used by tokio, reqwest, and other major crates
- **Example**:
  ```rust
  // Won't compile without storage provider
  let manager = CredentialManager::builder()
      .cache_ttl(Duration::from_secs(300))
      .build(); // ERROR: storage provider required
  
  // Correct usage
  let manager = CredentialManager::builder()
      .storage(storage_provider)
      .cache_ttl(Duration::from_secs(300))
      .build(); // OK
  ```

**Alternatives Considered**:
- **Runtime Validation**: Less safe, errors only at runtime
- **Required Parameters in `new()`**: Less flexible, harder to add optional config

#### 4. Batch Operation Strategy

**Decision**: Parallel execution with `tokio::task::JoinSet` and bounded concurrency

**Rationale**:
- **Performance**: Execute N operations concurrently (default 10 concurrent)
- **Backpressure**: Bounded concurrency prevents overwhelming storage providers
- **Error Handling**: Collect all results, don't fail-fast
- **Constitution Compliance**: Uses JoinSet per Principle IV (Async Discipline)

**Alternatives Considered**:
- **Sequential**: Too slow for large batches
- **Unbounded Parallelism**: Risk of overwhelming backend services
- **Stream-based**: More complex for simple batch operations

#### 5. Error Handling Strategy

**Decision**: Isolated `ManagerError` enum with context conversion from storage/cache errors

**Rationale**:
- **Constitution Compliance**: Principle II requires isolated errors per crate
- **Context Preservation**: Each error variant includes credential_id and operation context
- **Actionable Messages**: Error messages include specific failure reason
- **Example**:
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum ManagerError {
      #[error("Credential not found: {credential_id}")]
      NotFound { credential_id: CredentialId },
      
      #[error("Storage provider error for {credential_id}: {source}")]
      StorageError {
          credential_id: CredentialId,
          source: StorageError,
      },
      
      #[error("Cache error: {0}")]
      CacheError(String),
      
      #[error("Validation failed for {credential_id}: {reason}")]
      ValidationError {
          credential_id: CredentialId,
          reason: String,
      },
  }
  ```

**Alternatives Considered**:
- **Shared Error Crate**: Violates Constitution Principle II
- **Generic Error Type**: Loses context and type safety
- **anyhow::Error**: Too opaque for library code

### Technology Stack Summary

| Component | Technology | Version | Justification |
|-----------|-----------|---------|---------------|
| **Caching** | moka | 0.12 | LRU + TTL, async-first, mature |
| **Async Runtime** | tokio | 1.x | Already workspace dependency |
| **Error Handling** | thiserror | 1.x | Isolated errors per constitution |
| **Serialization** | serde | 1.x | Already workspace dependency |
| **Logging** | tracing | 0.1 | Structured observability |
| **Security** | zeroize | 1.8 | Zero-on-drop for secrets |

**New Dependencies to Add**:
- `moka = { version = "0.12", features = ["future"] }` - For caching layer

**No Breaking Changes**: All new dependencies are additive, existing code unaffected.

## Phase 1: Design Artifacts

**Status**: ✅ COMPLETED

### 1. Data Model (data-model.md)

See [data-model.md](./data-model.md) for complete type definitions including:
- `CredentialManager` - Core manager struct with storage and cache
- `ManagerConfig` - Configuration including storage, cache, scope settings
- `CacheConfig` - Cache-specific configuration (enabled, TTL, max size, strategy)
- `ManagerBuilder` - Typestate builder for safe construction
- `ManagerError` - Isolated error type with variants for each failure mode
- `ValidationResult` - Batch validation result with per-credential status
- `CacheStats` - Cache performance metrics (hits, misses, size, evictions)

### 2. API Contracts (contracts/)

See [contracts/manager-api.md](./contracts/manager-api.md) for:
- CRUD operations (store, retrieve, delete, list)
- Batch operations (store_batch, retrieve_batch, delete_batch)
- Scope operations (retrieve_scoped, list_scoped, filter_by_tags)
- Validation operations (validate, validate_batch)
- Cache operations (clear_cache, clear_cache_for, get_cache_stats)
- Configuration operations (builder pattern methods)

### 3. Quickstart Guide (quickstart.md)

See [quickstart.md](./quickstart.md) for 3-minute getting started tutorial covering:
- Basic CRUD operations (5 lines of code)
- Multi-tenant scope isolation example
- Cache configuration example
- Batch operations example
- Error handling patterns

### 4. Agent Context Updated

Agent context file (CLAUDE.md) updated with:
- New technologies: moka caching library
- Architecture: CredentialManager as facade over storage + cache
- Patterns: Builder pattern, batch operations, scope-based multi-tenancy
- Performance: Caching strategy, bounded parallelism for batches

## Phase 2: Implementation Tasks

**Status**: ⏳ PENDING - Use `/speckit.tasks` to generate actionable tasks

The task generation command will create `tasks.md` with:
- Dependency-ordered implementation tasks
- TDD workflow (tests first, then implementation)
- Parallel execution opportunities
- Verification checkpoints

**Task Categories** (Preview):
1. **Core Manager** - CredentialManager struct, CRUD operations
2. **Builder Pattern** - Typestate builder with compile-time safety
3. **Caching Layer** - moka integration with TTL and LRU
4. **Batch Operations** - Parallel execution with JoinSet
5. **Scope Isolation** - Multi-tenant filtering and access control
6. **Validation** - Single and batch validation logic
7. **Error Handling** - ManagerError conversions and context
8. **Observability** - Tracing, metrics, cache stats
9. **Integration Tests** - End-to-end testing with mock storage
10. **Documentation** - Rustdoc, examples, migration guide

## Test Strategy

### Unit Tests (TDD Required)

Write tests BEFORE implementation for each component:

**CredentialManager Core**:
- Store credential, retrieve by ID (basic CRUD)
- Delete credential, verify retrieval returns None
- List all credentials, verify IDs match
- Store duplicate ID, verify error

**Scope Isolation**:
- Store in scope A, retrieve from scope B returns None
- List scoped credentials, verify isolation
- Hierarchical scope inheritance tests
- Tag-based filtering within scope

**Caching**:
- Retrieve twice, second is cache hit (measure time)
- Update credential, cache invalidated
- Delete credential, cache invalidated
- TTL expiration, next retrieval is cache miss
- LRU eviction when cache full

**Batch Operations**:
- Store 100 credentials in batch, verify all persisted
- Retrieve batch, verify order and completeness
- Delete batch, verify all removed
- Batch with partial failures, verify error handling

**Validation**:
- Validate non-expired credential returns true
- Validate expired credential returns false
- Batch validation with mixed results
- Validation with missing expiration metadata

### Integration Tests

**With Mock Storage**:
- End-to-end CRUD workflows
- Multi-tenant scenarios with multiple scopes
- Cache hit/miss scenarios with real timing
- Batch operations under load (100+ credentials)
- Error scenarios (storage failures, timeouts)

**Time-Based Tests**:
```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_cache_ttl_expiration() {
    tokio::time::pause(); // Freeze time
    
    // Store and retrieve (cache miss)
    let cred = manager.retrieve(&id).await.unwrap();
    
    // Retrieve again (cache hit)
    let start = Instant::now();
    let cred2 = manager.retrieve(&id).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(5)); // Cache hit
    
    // Advance time past TTL
    tokio::time::advance(Duration::from_secs(301)).await;
    
    // Retrieve again (cache miss, refetch from storage)
    let cred3 = manager.retrieve(&id).await.unwrap();
    // Verify fetched from storage (longer latency)
}
```

### Performance Tests

**Cache Performance**:
- Measure cache hit latency (target: <5ms p99)
- Measure cache miss latency (target: <100ms p99)
- Verify cache hit rate >80% under 5:1 read/write ratio
- Memory usage stays bounded under load

**Batch Performance**:
- 100 credential batch vs 100 sequential operations
- Verify >50% time reduction
- Bounded concurrency prevents overwhelming storage

**Concurrency**:
- 10,000 concurrent retrieve operations
- No deadlocks or contention issues
- Linear scaling up to CPU core count

## Migration and Deployment

### Migration Path

**Phase 2 → Phase 3** (Current):
- Add moka dependency to Cargo.toml
- Implement manager module (new code, no breaking changes)
- Existing storage providers unchanged (backward compatible)
- Update prelude to export CredentialManager

**No Breaking Changes**:
- All Phase 1 and Phase 2 APIs remain unchanged
- CredentialManager is additive functionality
- Existing code using StorageProvider directly continues to work

### Deployment Strategy

**Feature Flags**: No new feature flags required
- Caching is configurable at runtime (not compile-time)
- Manager works with any existing storage provider

**Rollout**:
1. Merge manager implementation to main
2. Update examples and documentation
3. Migration guide for users of raw StorageProvider API
4. Next phase (Phase 4: Rotation) builds on CredentialManager

## Risks and Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|------------|
| **Cache Coherence** | Stale data after external updates | Medium | Invalidate cache on all mutations; document external update limitations |
| **Memory Exhaustion** | OOM if cache unbounded | Low | LRU eviction enforced; default 1000 entry limit; configurable max |
| **Storage Provider Latency** | Slow cache misses impact UX | Medium | Reasonable timeout defaults (5s); retry logic from Phase 2 |
| **Scope Validation** | Incorrect isolation breaks multi-tenancy | High | Comprehensive test suite; scope validation in all operations |
| **Builder API Breaking Changes** | Future config additions break builds | Low | Typestate design allows adding optional methods without breaking existing code |

## Success Criteria Mapping

Mapping spec success criteria (SC-001 to SC-012) to implementation:

- **SC-001** (3 lines of code): Builder pattern with sensible defaults
- **SC-002** (<10ms cache hits, <100ms misses): Moka cache + benchmarks
- **SC-003** (1000+ scopes): Scope filtering in all operations + integration tests
- **SC-004** (80% cache hit rate): TTL and LRU eviction + metrics tracking
- **SC-005** (50% batch improvement): Parallel execution with JoinSet
- **SC-006** (100% expired detection): Validation logic + time-based tests
- **SC-007** (10,000 concurrent ops): Thread-safe Arc/RwLock + concurrency tests
- **SC-008** (actionable errors): ManagerError with context + error message tests
- **SC-009** (5s storage failure detection): Timeout configuration + failure tests
- **SC-010** (bounded cache memory): LRU eviction + memory tests
- **SC-011** (encryption at rest): Inherited from Phase 2 storage providers
- **SC-012** (zeroization within 1s): Inherited from Phase 1 SecretString

## Next Steps

1. ✅ Phase 0 research complete (this section)
2. ✅ Phase 1 design artifacts complete (data-model.md, contracts/, quickstart.md)
3. ⏳ Run `/speckit.tasks` to generate dependency-ordered implementation tasks
4. ⏳ Execute tasks with TDD workflow (tests first)
5. ⏳ Verify all success criteria met
6. ⏳ Run constitution quality gates (fmt, clippy, check, doc)
7. ⏳ Mark phase complete, proceed to Phase 4 (Rotation) planning

---

**Plan Status**: ✅ READY FOR TASK GENERATION  
**Command**: `/speckit.tasks` to create actionable task breakdown  
**Estimated Effort**: 3-4 weeks (per roadmap Phase 3 estimate)
