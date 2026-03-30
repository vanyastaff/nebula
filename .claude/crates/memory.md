# nebula-memory
High-performance memory management — arenas, pools, LRU/TTL caching, memory pressure detection.

## Invariants
- All major subsystems are feature-gated: `arena`, `pool`, `cache`, `budget`, `monitoring`. Default: `std` + `pool` + `arena` + `cache` + `budget` + `logging`.
- `GlobalAllocatorManager` is lazily initialized via `LazyLock` — no `init()` call required.
- Error type `MemoryError` uses `Box<str>` for string fields to avoid excess heap capacity.
- Internal module is `foundation` (not `core`) to avoid shadowing `::core`.
- `ConcurrentComputeCache` uses random eviction (not LRU) — no access metadata tracked on reads.
- `ConcurrentComputeCache` has always-on atomic metrics via `stats()` — zero overhead on x86.
- `ConcurrentComputeCache` supports lazy TTL expiry on reads when `CacheConfig::ttl` is set.
- `CacheKey` trait requires `Hash + Eq + Clone + Send + Sync` (unified in `compute.rs`).
- Arena has a generation counter — positions from before `reset()` are rejected by `reset_to_position()`.
- `BumpAllocator` always uses `AtomicCursor` (CellCursor removed — unsound `Sync` impl).
- Budget `release_memory` propagates only the actually-released amount to parent (saturating_sub clamped).
- Budget lock ordering: `used` → `config` → `peak` → `stats` → `history`, child → parent.

## Key Decisions
- `Arena` allocates in bulk and frees everything at once — no per-item deallocation. Use for batch-scoped data.
- `ObjectPool` reuses objects on drop (RAII). Use for frequently allocated/freed objects (e.g., buffers).
- `ComputeCache` is the single-threaded LRU/TTL cache — uses all `CacheConfig` fields.
- `ConcurrentComputeCache` honors only `max_entries`, `initial_capacity`, `ttl` from `CacheConfig`. Other fields (`policy`, `track_metrics`, `load_factor`, `auto_cleanup`) are ignored.
- `MemoryBudget` enforces memory limits per context (requires `budget` feature).
- Error constructors are pure (no logging side-effects) — log at call site.
- `align_up` uses `wrapping_add` for overflow safety; `checked_align_up` returns `Option<usize>` for allocation paths.
- Arena `alloc_bytes_aligned` uses a loop instead of recursion to prevent stack overflow.
- Zero-size allocations return a dangling pointer (prevents aliasing with next real allocation).
- Stack allocator `try_pop` uses CAS instead of store to prevent concurrent corruption.

## Traps
- Uses `unsafe` internally (allocator, pool pointer alignment, syscalls) despite looking like a safe API. Don't add unsafe-free guarantees to documentation.
- `cast_ptr_alignment` is allowed deliberately — the pool allocator aligns by design.
- `monitoring` feature adds `MonitoredAllocator` — only enable if you need pressure callbacks, it adds overhead.
- `init()` is deprecated and is a no-op. Remove calls to it.
- Sealed traits (`AllocatorInternal`) prevent external implementations of internal allocator operations.
- `get_or_compute` on `ConcurrentComputeCache` may call `compute_fn` more than once for the same key under concurrent access — use only with idempotent functions.
- `ConcurrentComputeCache::max_entries` is a soft cap — under heavy concurrent contention, len may temporarily exceed capacity.
- `ThreadSafePool::clear()` inlines stats update to avoid self-deadlock (do NOT call `update_memory_stats()` while holding `inner` lock).

## Relations
- Optional dep on nebula-log (feature: `logging`), nebula-system for pressure detection.
- Only consumer: nebula-expression (uses `ConcurrentComputeCache`, `CacheConfig`, `CacheStats`, `MemoryError`).

<!-- reviewed: 2026-03-30 -->
