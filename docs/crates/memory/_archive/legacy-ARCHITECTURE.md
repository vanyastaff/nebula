# Architecture

## Module layout

`nebula-memory` is split into independent capability blocks:

1. `allocator/` - low-level allocators and allocator manager
2. `arena/` - arena-style allocations and scoped reset patterns
3. `pool/` - reusable object pools and pool health helpers
4. `cache/` - cache policies and multi-level cache composition
5. `budget/` - memory budget tracking, reservation, and policy control
6. `stats/` + `monitoring/` - observability and predictive tracking
7. `core/` + `error.rs` - shared traits/config/types/error contracts

## Runtime model

- Allocation-heavy code paths can use specialized allocators instead of global allocator.
- Repeated object creation paths use `pool` module for reuse.
- Expensive computed results use cache module with configurable eviction.
- Budget module can gate allocations and trigger pressure actions.
- Optional monitoring/stats provide operational insight without forcing overhead on minimal builds.

## Feature-gated design

Major modules are behind features (`pool`, `arena`, `cache`, `stats`, `budget`, `monitoring`, `async`, `logging`).

This keeps:
- default behavior practical for runtime
- advanced capabilities opt-in
- compile time and dependency surface controlled

## Safety boundaries

- core unsafe allocator internals are localized in allocator/arena components
- API surfaces rely on typed traits (`Allocator`, `TypedAllocator`, `Resettable`, `MemoryUsage`)
- integration with `nebula-log` and system-level calls is optional and isolated by feature/platform gates
