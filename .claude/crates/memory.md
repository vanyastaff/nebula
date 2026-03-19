# nebula-memory
High-performance memory management — arenas, pools, LRU/TTL caching, memory pressure detection.

## Invariants
- All major subsystems are feature-gated: `arena`, `pool`, `cache`, `budget`, `monitoring`. Default: `std` only.
- `init()` must be called at startup to initialize the global allocator manager.

## Key Decisions
- `Arena` allocates in bulk and frees everything at once — no per-item deallocation. Use for batch-scoped data.
- `ObjectPool` reuses objects on drop (RAII). Use for frequently allocated/freed objects (e.g., buffers).
- `ComputeCache` is the LRU/TTL cache — keyed by `CacheKey`.
- `MemoryBudget` enforces memory limits per context (requires `budget` feature).

## Traps
- Uses `unsafe` internally (allocator, pool pointer alignment, syscalls) despite looking like a safe API. Don't add unsafe-free guarantees to documentation.
- `cast_ptr_alignment` is allowed deliberately — the pool allocator aligns by design.
- `monitoring` feature adds `MonitoredAllocator` — only enable if you need pressure callbacks, it adds overhead.

## Relations
- Optional dep on nebula-log (feature: `logging`), nebula-system for pressure detection.
