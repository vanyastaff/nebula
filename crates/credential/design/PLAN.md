# Implementation Plan: nebula-credential

**Crate**: `nebula-credential` | **Path**: `crates/credential` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

Universal credential management system providing protocol-agnostic authentication flows (OAuth2, API Keys, JWT, SAML, Kerberos, mTLS), secure encrypted storage with multiple backends, and credential rotation with state machine invariants. Current focus is contract consolidation -- aligning docs with the actual codebase and defining stable API subsets.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (time, sync, macros, rt)
**Key Dependencies**: aes-gcm, argon2, zeroize, moka (cache), reqwest, tokio-util, nebula-core, nebula-log, nebula-parameter, nebula-eventbus, nebula-storage (optional)
**Feature Flags**: `storage-local` (default), `storage-aws`, `storage-vault`, `storage-k8s`, `storage-postgres`, `storage-all`
**Testing**: `cargo test -p nebula-credential`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract Consolidation | 🔄 In Progress | Unify docs, define stable API surface, formalize scope enforcement |
| Phase 2: Rotation Reliability | ⬜ Planned | Property-based tests, rollback/retry coverage, audit events |
| Phase 3: Provider Hardening | ⬜ Planned | Capability matrix, observability parity, migration tooling |
| Phase 4: Production Infrastructure | ⬜ Planned | Implement CredentialManager stubs, CredentialProvider, API endpoints |
| Phase 5: Security Hardening | ⬜ Planned | EncryptionProvider, AuditLogger, lifecycle state machine, strict scopes |
| Phase 6: Performance and Scale | ⬜ Planned | Benchmarks, L2 Redis cache, rate limiting, high-cardinality workloads |
| Phase 7: Protocol Completeness | ⬜ Planned | SAML 2.0, Kerberos, mTLS, JWT, proactive OAuth2 refresh |
| Phase 8: Toolchain and Compatibility | ⬜ Planned | Re-enable adapter module, schema versioning, error taxonomy |

## Phase Details

### Phase 1: Contract Consolidation

**Goal**: Unify documentation with the actual codebase, define stable external API subsets, and formalize scope enforcement behavior across all provider implementations.

**Deliverables**:
- Unified top-level crate docs and `docs/crates/credential/` narrative
- Stable external API subsets documented for manager/provider/rotation consumers
- Scope enforcement behavior documented and tested in all provider implementations

**Exit Criteria**:
- ARCHITECTURE.md, API.md, INTERACTIONS.md all reflect actual codebase
- Stable API surface documented in API.md with explicit stability annotations
- Scope enforcement behavior documented and tested in all provider impls
- `cargo doc --no-deps -p nebula-credential` generates clean output

**Risks**:
- API surface may be larger than expected, requiring difficult stability decisions
- Scope enforcement behavior may vary between providers in ways not yet documented

**Dependencies**: None

### Phase 2: Rotation Reliability

**Goal**: Strengthen state machine invariants and transition validation, add property-based and chaos-like tests for rollback/retry/grace-period paths, tighten audit event coverage.

**Deliverables**:
- Property-based tests (proptest) covering rotation state machine transitions
- Rollback path integration tests with injected failures at each phase
- Structured audit event for every rotation outcome

**Exit Criteria**:
- Property-based tests (proptest) cover rotation state machine transitions
- Rollback path has dedicated integration test with injected failures at each phase
- Every rotation outcome (success/rollback/timeout/grace-expiry) emits structured audit event
- Flaky test budget: zero (deterministic mocks)

**Risks**:
- State machine may have undiscovered edge cases in rollback paths
- Audit event coverage may require refactoring existing rotation code

**Dependencies**: Phase 1

### Phase 3: Provider Hardening

**Goal**: Standardize provider capability matrix and failure semantics, improve observability parity across backends, add migration tooling.

**Deliverables**:
- `capabilities() -> ProviderCapabilities` on all providers
- `StorageMetrics` populated uniformly across local/AWS/Vault/K8s backends
- Migration tooling between providers with consistency checks

**Exit Criteria**:
- All providers implement `capabilities() -> ProviderCapabilities`
- `StorageMetrics` populated uniformly across local/AWS/Vault/K8s backends
- `cargo test --features storage-local,storage-aws,storage-vault,storage-k8s` passes

**Risks**:
- Backend-specific behaviors may resist uniform abstraction
- Migration tooling adds surface area for data loss bugs

**Dependencies**: Phase 1

### Phase 4: Production Infrastructure

**Goal**: Replace manager stubs with working implementations, complete CredentialProvider, integrate with type registry and API layer.

**Deliverables**:
- `CredentialManager::create`, `continue_flow`, `list_types` implementations
- Full `CredentialProvider` implementation (type-based `credential<C>()`)
- `ProtocolRegistry` integration for dynamic type registration
- nebula-api integration for credential CRUD endpoints

**Exit Criteria**:
- `POST /credentials` with OAuth2 type returns 202 + redirect URL
- `POST /credentials/:id/callback` completes flow, returns 200 active credential
- `GET /credential-types` returns registered type schemas
- Type-based `credential<C>()` works with configured type registry

**Risks**:
- API integration introduces coupling with HTTP layer
- Interactive flow state management across requests is complex

**Dependencies**: Phase 1

### Phase 5: Security Hardening

**Goal**: Pluggable crypto backends, structured audit pipelines, compile-time state machine validation, strict scope enforcement mode.

**Deliverables**:
- EncryptionProvider trait for pluggable crypto backends (local AES, KMS, Vault Transit)
- AuditLogger trait for structured compliance pipelines
- Credential lifecycle state machine with validated transitions
- Strict scope enforcement mode as configuration option

**Exit Criteria**:
- KMS-backed encryption provider passes all crypto tests
- Audit events emitted as structured types, not log strings
- State machine rejects illegal transitions at compile time where possible
- Strict scope mode available as configuration option

**Risks**:
- KMS integration adds external service dependency to test suite
- Compile-time state machine validation may require significant type-level encoding

**Dependencies**: Phase 3

### Phase 6: Performance and Scale

**Goal**: Benchmark cache behavior, add L2 Redis cache for multi-node deployments, implement rate limiting, optimize for high-cardinality workloads.

**Deliverables**:
- Criterion benchmarks for manager cache hit/miss behavior
- L2 Redis cache for multi-node fleet deployments
- Rate limiting / access budgets
- High-cardinality tenant workload benchmarks

**Exit Criteria**:
- Criterion benchmarks committed; cache hit p99 < 10ms
- No regression from Phase 5 baseline
- High-cardinality benchmark (10K tenants x 100 credentials) documented
- L2 cache reduces storage load by >50% in fleet scenario

**Risks**:
- Redis dependency adds operational complexity
- Cache invalidation across nodes is notoriously hard

**Dependencies**: Phase 4

### Phase 7: Protocol Completeness

**Goal**: Replace protocol stubs with working implementations, add integration tests with mock IdP servers.

**Deliverables**:
- SAML 2.0 full implementation (signature verification, assertion validation)
- Kerberos TGT flow
- mTLS certificate management and rotation
- JWT validation and issuance
- Proactive OAuth2 token refresh

**Exit Criteria**:
- All protocol stubs replaced with working implementations
- Integration tests with mock IdP servers
- Documentation updated with DX examples for each protocol

**Risks**:
- Protocol specifications are complex and error-prone to implement
- Mock IdP servers may not cover real-world edge cases

**Dependencies**: Phase 4

### Phase 8: Toolchain and Compatibility

**Goal**: Re-enable adapter module, define compatibility guarantees for serialized formats, establish rotation policy versioning and unified error taxonomy.

**Deliverables**:
- `core::adapter` module re-enabled and tested
- Compatibility guarantees for serialized metadata and rotation policy schemas
- Rotation policy versioning
- Unified error taxonomy

**Exit Criteria**:
- `core::adapter` enabled and covered by tests
- Rotation policy schema has explicit version envelope
- MIGRATION.md updated with compatibility matrix for persisted formats
- Error codes documented and stable

**Risks**:
- Adapter module may need significant updates to match current trait design
- Versioning serialized schemas requires careful migration planning

**Dependencies**: Phase 5

## Timeline Estimate

| Phase | Focus | Estimate | Dependencies |
|-------|-------|----------|-------------|
| 1 | Contract Consolidation | 2-3 weeks | None |
| 2 | Rotation Reliability | 3-4 weeks | Phase 1 |
| 3 | Provider Hardening | 4-6 weeks | Phase 1 |
| 4 | Production Infrastructure | 4-5 weeks | Phase 1 |
| 5 | Security Hardening | 4-5 weeks | Phase 3 |
| 6 | Performance and Scale | 3-4 weeks | Phase 4 |
| 7 | Protocol Completeness | 5-6 weeks | Phase 4 |
| 8 | Toolchain and Compatibility | 2-3 weeks | Phase 5 |

**Total to v1.0:** ~28-36 weeks (7-9 months). Phases 2-4 can run partially in parallel. Phases 5-7 can run partially in parallel.

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-log`, `nebula-parameter`, `nebula-eventbus`, `nebula-storage` (optional)
- **Depended by**: `nebula-action`, `nebula-plugin`, `nebula-resource`, `nebula-sdk`

## Verification

- [ ] `cargo check -p nebula-credential`
- [ ] `cargo test -p nebula-credential`
- [ ] `cargo clippy -p nebula-credential -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-credential`
