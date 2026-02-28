# Decisions

## D001: Multi-paradigm memory toolkit in one crate

Status: Accepted

Keep allocators, object pools, cache, and budgeting together because runtime consumers often need a combined strategy.

## D002: Feature-gated modularity over one monolithic runtime mode

Status: Accepted

`pool`, `arena`, `cache`, `stats`, `budget`, `logging`, `async` remain independent features to avoid mandatory overhead.

## D003: Reuse-first approach

Status: Accepted

Primary optimization strategy is reuse (`ObjectPool`, allocator reset, cache hits) rather than raw allocation throughput only.

## D004: Optional observability

Status: Accepted

Stats/monitoring/tracing-like behavior must not be required for production runtime correctness.

## D005: Platform-aware optimizations remain additive

Status: Accepted

`numa-aware`, `linux-optimizations`, `simd` are optional performance enhancements, not baseline assumptions.
