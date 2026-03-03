# Roadmap

`nebula-credential` roadmap prioritizes security guarantees, operational reliability, and the path to a production-ready universal auth library.

## Phase 1: Contract Consolidation (Current)

**Goals:**
- Unify top-level crate docs and `docs/crates/credential/` narrative
- Define stable external API subsets for manager/provider/rotation consumers
- Formalize scope enforcement behavior across all provider implementations

**Exit criteria:**
- ARCHITECTURE.md, API.md, INTERACTIONS.md all reflect actual codebase
- Stable API surface documented in API.md with explicit stability annotations
- Scope enforcement behavior documented and tested in all provider impls
- `cargo doc --no-deps -p nebula-credential` generates clean output

## Phase 2: Rotation Reliability

**Goals:**
- Strengthen state machine invariants and transition validation (P-009)
- Add property-based and chaos-like tests for rollback/retry/grace-period paths
- Tighten audit event coverage for every rotation outcome

**Exit criteria:**
- Property-based tests (proptest) cover rotation state machine transitions
- Rollback path has dedicated integration test with injected failures at each phase
- Every rotation outcome (success/rollback/timeout/grace-expiry) emits structured audit event
- Flaky test budget: zero (deterministic mocks)

## Phase 3: Provider Hardening

**Goals:**
- Standardize provider capability matrix (P-001) and failure semantics
- Improve observability parity across backends (metrics/traces/error codes)
- Add migration tooling between providers with consistency checks

**Exit criteria:**
- All providers implement `capabilities() -> ProviderCapabilities`
- `StorageMetrics` populated uniformly across local/AWS/Vault/K8s backends
- `cargo test --features storage-local,storage-aws,storage-vault,storage-k8s` passes

## Phase 4: Production Infrastructure

**Goals:**
- Implement `CredentialManager::create`, `continue_flow`, `list_types` (remove stubs)
- Full `CredentialProvider` implementation (type-based `credential<C>()`)
- `ProtocolRegistry` integration for dynamic type registration (P-003 type registry)
- nebula-api integration for credential CRUD endpoints

**Exit criteria:**
- `POST /credentials` with OAuth2 type returns 202 + redirect URL
- `POST /credentials/:id/callback` completes flow, returns 200 active credential
- `GET /credential-types` returns registered type schemas
- Type-based `credential<C>()` works with configured type registry

## Phase 5: Security Hardening

**Goals:**
- EncryptionProvider trait (P-007) for pluggable crypto backends (local AES, KMS, Vault Transit)
- AuditLogger trait (P-006) for structured compliance pipelines
- Credential lifecycle state machine (P-009) with validated transitions
- Strict scope enforcement mode (P-002)

**Exit criteria:**
- KMS-backed encryption provider passes all crypto tests
- Audit events emitted as structured types, not log strings
- State machine rejects illegal transitions at compile time where possible
- Strict scope mode available as configuration option

## Phase 6: Performance and Scale

**Goals:**
- Benchmark manager cache hit/miss behavior under load (criterion)
- L2 Redis cache (P-008) for multi-node fleet deployments
- Rate limiting / access budgets (P-004)
- Optimize high-cardinality tenant workloads

**Exit criteria:**
- Criterion benchmarks committed; cache hit p99 < 10ms
- No regression from Phase 5 baseline
- High-cardinality benchmark (10K tenants × 100 credentials) documented
- L2 cache reduces storage load by >50% in fleet scenario

## Phase 7: Protocol Completeness

**Goals:**
- SAML 2.0 full implementation (signature verification, assertion validation)
- Kerberos TGT flow
- mTLS certificate management and rotation
- JWT validation and issuance
- Proactive OAuth2 token refresh (P-010)

**Exit criteria:**
- All protocol stubs replaced with working implementations
- Integration tests with mock IdP servers
- Documentation updated with DX examples for each protocol

## Phase 8: Toolchain and Compatibility

**Goals:**
- Re-enable `core::adapter` module (TODO in source)
- Define compatibility guarantees for serialized metadata and rotation policy schemas
- Rotation policy versioning (P-003)
- Unified error taxonomy (P-005)

**Exit criteria:**
- `core::adapter` enabled and covered by tests
- Rotation policy schema has explicit version envelope
- MIGRATION.md updated with compatibility matrix for persisted formats
- Error codes documented and stable

## Timeline Estimate

| Phase | Focus | Estimate | Dependencies |
|-------|-------|----------|-------------|
| 1 | Contract Consolidation | 2–3 weeks | None |
| 2 | Rotation Reliability | 3–4 weeks | Phase 1 |
| 3 | Provider Hardening | 4–6 weeks | Phase 1 |
| 4 | Production Infrastructure | 4–5 weeks | Phase 1 |
| 5 | Security Hardening | 4–5 weeks | Phase 3 |
| 6 | Performance and Scale | 3–4 weeks | Phase 4 |
| 7 | Protocol Completeness | 5–6 weeks | Phase 4 |
| 8 | Toolchain and Compatibility | 2–3 weeks | Phase 5 |

**Total to v1.0:** ~28–36 weeks (7–9 months)

Phases 2–4 can run partially in parallel. Phases 5–7 can run partially in parallel.
