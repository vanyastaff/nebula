# Roadmap

`nebula-credential` roadmap prioritizes security guarantees, operational reliability, and performance.

## Phase 1: Contract Consolidation

- unify top-level crate docs and `crates/credential/docs` narrative
- define stable external API subsets for manager/provider/rotation consumers
- formalize scope enforcement behavior across all provider implementations

## Phase 2: Rotation Reliability

- strengthen state machine invariants and transition validation
- add property-based and chaos-like tests for rollback/retry/grace-period paths
- tighten audit event coverage for every rotation outcome

## Phase 3: Provider Hardening

- standardize provider capability matrix and failure semantics
- improve observability parity across backends (metrics/traces/error codes)
- add migration tooling between providers with consistency checks

## Phase 4: Performance and Scale

- benchmark manager cache hit/miss behavior under load
- optimize high-cardinality tenant workloads
- reduce lock contention and serialization overhead in hot paths

## Phase 5: Toolchain and Compatibility

- workspace baseline today: Rust `1.92`
- prepare controlled migration to Rust `1.93+`
- define compatibility guarantees for serialized metadata and rotation policy schemas
