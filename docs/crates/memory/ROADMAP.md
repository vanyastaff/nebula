# Roadmap

## R1: API coherence pass

- align naming across allocator/pool/cache config builders
- reduce duplicated stats type names between modules
- document stable API subset intended for external crates

## R2: Runtime safety and correctness

- expand stress/property tests for concurrent pool/cache paths
- add explicit invariants for unsafe allocator internals
- verify shutdown semantics for async + monitoring combinations

## R3: Performance profile hardening

- benchmark suites per allocator strategy and workload pattern
- clarify when to use allocator-level pool vs object pool abstractions
- tune default configs for realistic Nebula workflow workloads

## R4: Integration ergonomics

- improve integration guides with `nebula-core` and runtime contexts
- standardize instrumentation hooks for `nebula-log`/metrics backends
- provide canonical examples for high-throughput action execution

## R5: Incremental advanced features

- mature adaptive pressure policies
- stabilize async support surface
- evaluate split of optional experimental APIs into sibling crates if surface becomes too large
