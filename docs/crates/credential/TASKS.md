# Tasks: nebula-credential

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `CRD`

---

## Phase 1: Contract Consolidation 🔄

**Goal**: Align docs with codebase, define stable API surface, formalize scope enforcement.

- [ ] CRD-T001 [P] Audit ARCHITECTURE.md against actual codebase — fix any drift in module boundaries
- [ ] CRD-T002 [P] Audit API.md — add explicit stability annotations (`stable`, `unstable`, `internal`) to every public item
- [ ] CRD-T003 [P] Audit INTERACTIONS.md — verify all integration points with nebula-action, nebula-resource, nebula-storage
- [ ] CRD-T004 Write scope enforcement tests for every provider impl (local, AWS, Vault, K8s)
- [ ] CRD-T005 Run `cargo doc --no-deps -p nebula-credential` and fix all warnings/missing docs
- [ ] CRD-T006 [P] Add doc-tests for manager/provider/rotation public API examples

**Checkpoint**: ARCHITECTURE.md, API.md, INTERACTIONS.md all reflect actual code; `cargo doc` clean.

---

## Phase 2: Rotation Reliability ⬜

**Goal**: Property-based tests for rotation state machine, full rollback coverage, structured audit events.

- [ ] CRD-T007 [P] Add proptest dependency; write property-based tests for rotation state machine transitions in `tests/rotation_proptest.rs`
- [ ] CRD-T008 Create integration test for rollback path with injected failures at each rotation phase in `tests/rotation_rollback.rs`
- [ ] CRD-T009 Ensure every rotation outcome emits structured audit event (success, rollback, timeout, grace-expiry) — replace any log strings
- [ ] CRD-T010 [P] Replace all non-deterministic mocks with deterministic fakes; verify zero flaky tests in CI

**Checkpoint**: proptest suite green; rollback integration test passes with injected failures; all audit events are structured types.

---

## Phase 3: Provider Hardening ⬜

**Goal**: Uniform capability matrix across providers, observability parity, migration tooling.

- [ ] CRD-T011 [P] Add `capabilities() -> ProviderCapabilities` to every provider (local, AWS, Vault, K8s, Postgres)
- [ ] CRD-T012 [P] Populate `StorageMetrics` in every backend (current, rotation count, error rates)
- [ ] CRD-T013 Write migration tool between providers with consistency checks in `src/migration.rs`
- [ ] CRD-T014 Add CI job: `cargo test -p nebula-credential --features storage-local,storage-aws,storage-vault,storage-k8s`

**Checkpoint**: All providers expose `capabilities()`; `StorageMetrics` populated uniformly; all-features CI passes.

---

## Phase 4: Production Infrastructure ⬜

**Goal**: Replace manager stubs, complete CredentialProvider, integrate with ProtocolRegistry and API.

- [ ] CRD-T015 Implement `CredentialManager::create` — full credential creation flow (not stub)
- [ ] CRD-T016 Implement `CredentialManager::continue_flow` — OAuth2/interactive callback handling
- [ ] CRD-T017 Implement `CredentialManager::list_types` — returns registered type schemas
- [ ] CRD-T018 Implement type-based `credential<C>()` on `CredentialProvider` with type registry
- [ ] CRD-T019 Integrate `ProtocolRegistry` for dynamic type registration (`src/protocol/registry.rs`)
- [ ] CRD-T020 Add nebula-api endpoints: `POST /credentials`, `POST /credentials/:id/callback`, `GET /credential-types`
- [ ] CRD-T021 Integration test: OAuth2 flow end-to-end (POST → 202 + redirect → callback → 200 active)

**Checkpoint**: POST /credentials with OAuth2 returns 202; callback completes flow; GET /credential-types returns schemas.

---

## Phase 5: Security Hardening ⬜

**Goal**: Pluggable crypto backends, structured audit logger, compile-time state machine, strict scope mode.

- [ ] CRD-T022 Define `EncryptionProvider` trait in `src/crypto/mod.rs`
- [ ] CRD-T023 Implement KMS-backed encryption provider (`src/crypto/kms.rs`)
- [ ] CRD-T024 Implement Vault Transit encryption provider (`src/crypto/vault_transit.rs`)
- [ ] CRD-T025 [P] Define `AuditLogger` trait for structured compliance pipelines in `src/audit.rs`
- [ ] CRD-T026 Encode credential lifecycle state machine as types — reject illegal transitions at compile time
- [ ] CRD-T027 Add strict scope enforcement mode as config option; document in SECURITY.md

**Checkpoint**: KMS provider passes all crypto tests; audit events are typed structs; strict scope mode configurable.

---

## Phase 6: Performance and Scale ⬜

**Goal**: Criterion benchmarks, L2 Redis cache, rate limiting, high-cardinality workload support.

- [ ] CRD-T028 [P] Add criterion benchmarks for manager cache hit/miss in `benches/cache.rs`
- [ ] CRD-T029 Implement L2 Redis cache for multi-node fleet deployments (`src/cache/redis.rs`)
- [ ] CRD-T030 [P] Implement rate limiting / access budgets per credential (`src/ratelimit.rs`)
- [ ] CRD-T031 Add high-cardinality benchmark: 10K tenants × 100 credentials in `benches/high_cardinality.rs`
- [ ] CRD-T032 Verify: cache hit p99 < 10ms; L2 cache reduces storage load by >50% in fleet scenario

**Checkpoint**: Criterion benchmarks committed; p99 < 10ms; Redis cache integrated.

---

## Phase 7: Protocol Completeness ⬜

**Goal**: Implement all protocol stubs — SAML 2.0, Kerberos, mTLS, JWT, proactive OAuth2 refresh.

- [ ] CRD-T033 Implement SAML 2.0 — signature verification + assertion validation in `src/protocols/saml.rs`
- [ ] CRD-T034 Implement Kerberos TGT flow in `src/protocols/kerberos.rs`
- [ ] CRD-T035 Implement mTLS certificate management + rotation in `src/protocols/mtls.rs`
- [ ] CRD-T036 [P] Implement JWT validation + issuance in `src/protocols/jwt.rs`
- [ ] CRD-T037 [P] Implement proactive OAuth2 token refresh (before expiry) in `src/protocols/oauth2.rs`
- [ ] CRD-T038 Add integration tests for each protocol using mock IdP servers
- [ ] CRD-T039 Add DX examples for each protocol in `docs/crates/credential/`

**Checkpoint**: All protocol stubs replaced; mock IdP integration tests pass; examples in docs.

---

## Phase 8: Toolchain and Compatibility ⬜

**Goal**: Re-enable adapter module, schema versioning, rotation policy versioning, unified error taxonomy.

- [ ] CRD-T040 Re-enable `core::adapter` module — fix all TODOs and add tests
- [ ] CRD-T041 Add explicit version envelope to rotation policy schema
- [ ] CRD-T042 Update MIGRATION.md with compatibility matrix for all persisted formats
- [ ] CRD-T043 [P] Define and document stable error code taxonomy in `src/error.rs`
- [ ] CRD-T044 Add serialization compatibility tests for persisted credential metadata

**Checkpoint**: `core::adapter` enabled and tested; rotation policy schema versioned; error codes stable and documented.

---

## Dependencies & Execution Order

- Phase 1 unblocks all other phases
- Phases 2, 3, 4 can run in parallel after Phase 1
- Phase 5 depends on Phase 3 (provider hardening)
- Phase 6 depends on Phase 4 (production infra)
- Phase 7 depends on Phase 4 (production infra)
- Phase 8 depends on Phase 5

## Verification (after all phases)

- [ ] `cargo check -p nebula-credential --all-features`
- [ ] `cargo test -p nebula-credential --features storage-local,storage-aws,storage-vault,storage-k8s`
- [ ] `cargo clippy -p nebula-credential -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-credential`
- [ ] `cargo bench -p nebula-credential`
