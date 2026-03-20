# nebula-memory
High-performance memory management — arenas, pools, LRU/TTL caching, memory pressure detection.

## Invariants
- All major subsystems are feature-gated: `arena`, `pool`, `cache`, `budget`, `monitoring`. Default: `std` + `pool` + `arena` + `cache` + `budget` + `logging`.
- `GlobalAllocatorManager` is lazily initialized via `LazyLock` — no `init()` call required.
- Error type `MemoryError` uses `Box<str>` for string fields to avoid excess heap capacity.
- Internal module is `foundation` (not `core`) to avoid shadowing `::core`.

## Key Decisions
- `Arena` allocates in bulk and frees everything at once — no per-item deallocation. Use for batch-scoped data.
- `ObjectPool` reuses objects on drop (RAII). Use for frequently allocated/freed objects (e.g., buffers).
- `ComputeCache` is the LRU/TTL cache — keyed by `CacheKey`.
- `MemoryBudget` enforces memory limits per context (requires `budget` feature).
- `AllocError`/`AllocResult` aliases removed — use `MemoryError`/`MemoryResult` directly.
- `MemoryManager` trait removed (zero implementations) — use `Allocator` trait instead.
- Error constructors are pure (no logging side-effects) — log at call site.

## Traps
- Uses `unsafe` internally (allocator, pool pointer alignment, syscalls) despite looking like a safe API. Don't add unsafe-free guarantees to documentation.
- `cast_ptr_alignment` is allowed deliberately — the pool allocator aligns by design.
- `monitoring` feature adds `MonitoredAllocator` — only enable if you need pressure callbacks, it adds overhead.
- `init()` is deprecated and is a no-op. Remove calls to it.
- Sealed traits (`AllocatorInternal`) prevent external implementations of internal allocator operations.

## Relations
- Optional dep on nebula-log (feature: `logging`), nebula-system for pressure detection.
- Only consumer: nebula-expression (uses `ConcurrentComputeCache`, `CacheConfig`, `CacheMetrics`, `MemoryError`).

<!-- reviewed: 2026-03-20 -->
