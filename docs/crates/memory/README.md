# nebula-memory

High-performance memory primitives for Nebula runtime workloads.

## Scope

- In scope:
  - specialized allocators (`bump`, `pool`, `stack`) and allocator manager
  - object pools, arenas, cache modules, memory budgets
  - optional stats/monitoring/profiling and async helpers
  - platform-aware optimization hooks (`numa-aware`, `simd`, `linux-optimizations`)
- Out of scope:
  - workflow orchestration and retries
  - credential lifecycle and secret policy ownership
  - distributed memory coordination across nodes

## Current State

- maturity: feature-rich crate with production-oriented internals and broad module surface.
- key strengths:
  - composable feature flags keep baseline lean
  - explicit error taxonomy (`MemoryError`) and retryability classification
  - practical primitives for reuse-first workloads (pools, arenas, caches)
  - integration-ready monitor and budgeting APIs
- key risks:
  - large API footprint increases integration and migration complexity
  - optional modules require stricter compatibility matrix testing

## Target State

- production criteria:
  - stable crate contracts for runtime/action integration
  - deterministic behavior under pressure and concurrent contention
  - clear guidance for module selection by workload type
  - measurable safety and performance regression gates
- compatibility guarantees:
  - additive APIs/features in minor releases
  - behavioral changes in error semantics, feature defaults, and critical contracts only in major releases

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
