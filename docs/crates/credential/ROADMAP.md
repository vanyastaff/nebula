# Roadmap

`nebula-credential` roadmap prioritizes security guarantees, operational reliability, and performance.

## Phase 1: Contract Consolidation

**Goals:**
- unify top-level crate docs and `crates/credential/docs` narrative
- define stable external API subsets for manager/provider/rotation consumers
- formalize scope enforcement behavior across all provider implementations

**Exit criteria:**
- ARCHITECTURE.md, API.md, INTERACTIONS.md all reflect actual codebase
- Stable API surface documented in API.md with explicit stability annotations
- Scope enforcement behavior documented and tested in all provider impls
- `cargo doc --no-deps -p nebula-credential` generates clean output

## Phase 2: Rotation Reliability

**Goals:**
- strengthen state machine invariants and transition validation
- add property-based and chaos-like tests for rollback/retry/grace-period paths
- tighten audit event coverage for every rotation outcome

**Exit criteria:**
- Property-based tests (proptest) cover rotation state machine transitions
- Rollback path has dedicated integration test with injected failures at each phase
- Every rotation outcome (success/rollback/timeout/grace-expiry) emits a structured audit event
- Flaky test budget: zero (deterministic mocks)

## Phase 3: Provider Hardening

**Goals:**
- standardize provider capability matrix and failure semantics
- improve observability parity across backends (metrics/traces/error codes)
- add migration tooling between providers with consistency checks

**Exit criteria:**
- All providers implement a `capabilities() -> ProviderCapabilities` method
- `StorageMetrics` populated uniformly across local/AWS/Vault/K8s backends
- `cargo test --features storage-local,storage-aws,storage-vault,storage-k8s` passes

## Phase 4: Performance and Scale

**Goals:**
- benchmark manager cache hit/miss behavior under load
- optimize high-cardinality tenant workloads
- reduce lock contention and serialization overhead in hot paths

**Exit criteria:**
- Criterion benchmarks committed; cache hit p99 < 10ms
- No regression from Phase 3 baseline
- High-cardinality benchmark (10k tenants × 100 credentials) documented

## Phase 5: Toolchain and Compatibility

**Goals:**
- workspace baseline today: Rust `1.93`
- re-enable `core::adapter` module (TODO in source)
- define compatibility guarantees for serialized metadata and rotation policy schemas

**Exit criteria:**
- `core::adapter` enabled and covered by tests
- Rotation policy schema has explicit version envelope (P-003 implemented)
- MIGRATION.md updated with compatibility matrix for persisted formats
