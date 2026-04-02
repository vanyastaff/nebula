# nebula-memory
High-performance memory management — arenas, pools, LRU/TTL caching, memory pressure detection.

## Invariants
- All major subsystems are feature-gated: `arena`, `pool`, `cache`, `budget`, `monitoring`. Default: `std` + `pool` + `arena` + `cache` + `budget` + `logging`.
- `GlobalAllocatorManager` is lazily initialized via `LazyLock` — no `init()` call required.
- `MemoryError` uses `#[derive(nebula_error::Classify)]` — no manual impl. Consumers must `use nebula_error::Classify` to call `.code()` / `.is_retryable()`.
- Error type `MemoryError` uses `Box<str>` for string fields to avoid excess heap capacity.
- Internal module is `foundation` (not `core`) to avoid shadowing `::core`.
- `ConcurrentComputeCache` uses random eviction (not LRU) — no access metadata tracked on reads.
- `ConcurrentComputeCache` has always-on atomic metrics via `stats()` — zero overhead on x86.
- `ConcurrentComputeCache` supports lazy TTL expiry on reads when `CacheConfig::ttl` is set.
- `CacheKey` trait requires `Hash + Eq + Clone + Send + Sync` (unified in `compute.rs`).
- Arena has a generation counter — positions from before `reset()` are rejected by `reset_to_position()`.
- `BumpAllocator` stores `AtomicCursor` as a concrete field — NOT `Box<dyn Cursor>`. The `Cursor` trait was removed; `AtomicCursor` is the only implementation. This eliminates vtable dispatch in the hot alloc path (−21% measured).
- `PoolAllocator` stores `block_pow2_mask: Option<usize>` — precomputed `block_size - 1` for power-of-2 block sizes. `deallocate_block` uses bitmask instead of `%` when `Some` (−14% measured on reuse benchmark). Falls back to modulo for non-power-of-2 sizes.
- Budget `release_memory` propagates only the actually-released amount to parent (saturating_sub clamped).
- Budget lock ordering: `used` → `config` → `peak` → `stats` → `history`, child → parent.

## Key Decisions
- `Arena` allocates in bulk and frees everything at once — no per-item deallocation. Use for batch-scoped data.
- `ObjectPool` reuses objects on drop (RAII). Use for frequently allocated/freed objects (e.g., buffers).
- `ComputeCache` is the single-threaded LRU/TTL cache — uses all `CacheConfig` fields.
- `ConcurrentComputeCache` honors only `max_entries`, `initial_capacity`, `ttl` from `CacheConfig`. Other fields (`policy`, `track_metrics`, `load_factor`, `auto_cleanup`) are ignored.
- `MemoryBudget` enforces memory limits per context (requires `budget` feature).
- Error constructors are pure (no logging side-effects) — log at call site.
- `not_supported()` constructor takes `&'static str` and creates `NotSupported` variant (not `InvalidState`).
- `align_up` uses `wrapping_add` for overflow safety; `checked_align_up` returns `Option<usize>` for allocation paths.
- Arena `alloc_bytes_aligned` uses a loop instead of recursion to prevent stack overflow.
- Zero-size allocations return a dangling pointer (prevents aliasing with next real allocation).
- Stack allocator `try_pop` uses CAS instead of store to prevent concurrent corruption.
- `HierarchicalPooledValue` uses `Arc<Mutex<HierarchicalPool>>` (not raw pointer) for thread-safe Drop. **Breaking**: `HierarchicalPool::get()` is now an associated fn taking `&Arc<Mutex<Self>>`.
- `TypedArena::alloc_slice` guarantees contiguous allocation — allocates new chunk if needed to fit all elements.
- `AllocatorManager::with_allocator` uses RAII guard for panic-safe allocator restore.
- `Batch<T>` is intentionally `!Send` — holds raw `*mut BatchAllocator<T>`; Drop accesses it without any mutex.  Use `into_vec()` to extract objects for cross-thread work.
- `GlobalBudgetManager` singleton uses `OnceLock<Arc<...>>` — no `unsafe` statics. `initialize()` is idempotent if called after `instance()`.
- `ArenaString::as_str()` returns `Result<&str, Utf8Error>` — breaking change from unchecked version, necessary because `push()` allows arbitrary bytes.
- `PoolBox<'a, T>` has a lifetime `'a` tied to `&'a PoolAllocator` — prevents allocator being dropped while a PoolBox is still alive. Breaking: all usages must thread the lifetime.
- `MemoryReservation::cancel()` acquires `claimed` before `canceled` — same order as `claim()`, prevents deadlock under concurrent claim+cancel.
- `StackAllocator::reallocate` in-place extension uses `checked_add` — prevents overflow wraparound bypassing the `end_addr` guard.

## Traps
- Uses `unsafe` internally (allocator, pool pointer alignment, syscalls) despite looking like a safe API. Don't add unsafe-free guarantees to documentation.
- `cast_ptr_alignment` is allowed deliberately — the pool allocator aligns by design.
- `monitoring` feature adds `MonitoredAllocator` — only enable if you need pressure callbacks, it adds overhead.
- `init()` is deprecated and is a no-op. Remove calls to it.
- Sealed traits (`AllocatorInternal`) prevent external implementations of internal allocator operations.
- `get_or_compute` on `ConcurrentComputeCache` may call `compute_fn` more than once for the same key under concurrent access — use only with idempotent functions.
- `ConcurrentComputeCache::max_entries` is a soft cap — under heavy concurrent contention, len may temporarily exceed capacity.
- `ThreadSafePool::clear()` inlines stats update to avoid self-deadlock (do NOT call `update_memory_stats()` while holding `inner` lock).
- `ThreadSafePool` uses `std::sync::Mutex` (not `parking_lot`) because it needs `Condvar::wait_timeout` for blocking `get_timeout`.
- `BumpAllocator::restore(&self)` is not thread-safe despite taking `&self` — caller must ensure no concurrent allocations during restore.
- `BumpConfig::thread_safe` only controls CAS backoff spin, NOT actual thread-safety (AtomicCursor is always used).
- `MemoryBudget::can_allocate` is advisory — TOCTOU race under concurrency. Use `request_memory` directly for correctness.
- `BudgetState` and `PressureAction` are `#[non_exhaustive]` — match with wildcard arm.
- `TypedArena::chunk_capacity` doubles via `saturating_mul` — cannot overflow.
- `set_active_allocator!` macro panics on unregistered ID — use `set_active_allocator()` method directly in production.
- All arena `allocate_chunk` growth-factor f64 casts are capped with `.min(max_chunk_size as f64)` before `as usize`.

## Relations
- Optional dep on nebula-log (feature: `logging`), nebula-system for pressure detection.
- Only consumer: nebula-expression (uses `ConcurrentComputeCache`, `CacheConfig`, `CacheStats`, `MemoryError`).

- `memory_prefetch` is `unsafe fn` — dereferences `*const u8` via `read_volatile`; was previously missing `unsafe` keyword (soundness hole).
- `LruPolicy::record_insertion` removes old node before pushing new one for duplicate keys — prevents orphaned nodes + unbounded `nodes` Vec growth.
- `LfuPolicy::record_insertion` removes from old frequency bucket and `access_order` before reinserting for duplicate keys — same orphaned-entry pattern as LRU.
- `BumpScope::Drop` uses `debug_assert!(result.is_ok())` — same pattern as `StackFrame` after checkpoint restore.
- `TtlPooledValue<T>` is intentionally `!Send` — holds `*mut TtlPool<T>`, Drop calls `return_object` without lock; same reasoning as `Batch<T>`. Use `detach()` to cross thread boundaries.
- `ConcurrentComputeCache::contains_key` checks TTL — consistent with `get()`. Old impl returned `true` for expired entries.

- `MemoryBudget::metrics()` computes `state()` BEFORE acquiring `stats` lock — avoids `used` re-acquisition while `stats` is held (deadlock with `request_memory`).
- `MemoryBudget::can_allocate` uses `saturating_add` — prevents silent overflow reporting false availability.
- `TtlPooledValue<T>` and `PriorityPooledValue<T>` are intentionally `!Send` — raw `*mut Pool` Drop without synchronization. Same pattern as `Batch<T>`.
- `ScheduledCache` background thread acquires locks in `cache` → `ttls` order, consistent with all other methods. Old code acquired `ttls` → `cache` in the background thread but `cache` → `ttls` everywhere else — classic deadlock.
- `ArenaGuard::Drop` uses `debug_assert!(result.is_ok())` on `reset_to_position` — surfaces misuse (position from different arena) during tests. Same pattern as `BumpScope` and `StackFrame`.

- `foundation::config` subsystem-level wrapper types are named `*Settings` (`PoolSettings`, `CacheSettings`, `BudgetSettings`, `ArenaSettings`, `StatsSettings`) — distinct from the operational configs (`pool::PoolConfig`, `cache::config::CacheConfig`, `budget::config::BudgetConfig`) to prevent naming collision.
- `MemoryUsage::total_memory()` uses `checked_add` — returns `None` on overflow, consistent with "total unknown" semantics.
- `MetricsExtension` uses split storage (`by_name` index + slot vector + free list), not a single `BTreeMap<String, MemoryMetric>`. Registration is slot upsert/reuse to avoid payload movement.
- `MetricsExtension` slot storage keeps metric payload without duplicating `name`; `name` lives in the index key. This reduces register-path cloning/allocation pressure.
- `MetricsStorage::remove` uses `BTreeMap::remove_entry` so unregister can move the owned key into the returned `MemoryMetric` without extra string clone.
- `MetricsExtension::register_metric` now takes `&self` and mutates interior `RwLock` state. Callers no longer need mutable extension access.
- `global_metrics()` now returns a cloned `MetricsExtension` handle sharing the same reporter/storage (`Arc` + interior lock), and no longer reconstructs state through a noop delegating reporter.
- `MetricsExtension` has explicit `unregister_metric(&self, name)` that tombstones slot storage and reuses slot ids via free-list.
- Metrics reporter dispatch is now an enum adapter (`Noop`, `Debug`, `Custom`) so built-in reporters avoid trait-object dispatch.
- Metrics storage synchronization remains `RwLock` after phase-2.5 experiments: `HashMap`/`Mutex` variant increased `register_metric` assembly complexity with no measured win.
- Added dedicated benchmark target `metrics_extension_benchmarks` for register/update/unregister throughput and adapter-mode comparison (`noop_adapter` vs `custom_dyn`).

- `PriorityPool::return_object` uses `iter().min_by_key()` + `Vec::remove` to find and evict the MINIMUM-priority element — `BinaryHeap::peek()` returns the max, the old code was backwards.
- `TtlPool` has its own `created_count: usize` field for `max_capacity` enforcement — same pattern as `PriorityPool`. The old code read `stats.total_created()` which returned 0 when the `stats` feature was disabled, making bounded pools unbounded.
- `LfuPolicy::record_windowed_access` uses `checked_sub(...).unwrap_or(now)` for the cutoff — old code used `.unwrap()` which panicked on startup/in tests when the clock reading was less than `time_window`.
- `LfuPolicy::simple_hash` uses `DefaultHasher::default()` (fixed seed) — old code used `RandomState::new()` per call, making the hash key-independent and defeating probabilistic counting.
- `CacheConfig::for_time_sensitive` uses `ttl / 4` — old code used `Duration::from_secs(ttl.as_secs() / 4)` which produced `Duration::ZERO` for any TTL < 4 seconds, failing `validate()`.
- `AtomicCacheStats::record_deletion` uses `fetch_update` with `saturating_sub` — old code used `fetch_sub` which wraps to `u64::MAX` on underflow, unlike `StatsCollector` which saturates.
- `BudgetConfig::effective_limit` for `Percentage(pct)` uses `limit.saturating_mul(pct) / 100` — old code did `limit / 100 * pct` (divide-first) which produced 0 overcommit for any `limit < 100`.
- `FairSharePolicy::suggest_reclaim` guards `limit == 0` before computing `used / limit` — old code computed `Inf as u8` (implementation-defined, 0 on x86).
- `TypedArena::reset` walks the prepended chunk list to its tail, keeping only the oldest chunk and dropping all newer ones — old code set `current_chunk` to the head (newest), leaving older chunks permanently unreachable and leaking memory across reset cycles.
- `budget::budget::metrics()` renames local `stats` binding to `alloc_stats` — avoids `clippy::similar_names` with the adjacent `state` binding.
- `allocator/stack/allocator.rs` in-place reallocate collapses nested `if-let` + `if` + `if` into a single `if let … && … && …` — resolves clippy nesting and collapsible-if lints.

- `MemoryStats::record_deallocation` uses `fetch_update` with `saturating_sub` — same pattern as `AtomicCacheStats`. Old `fetch_sub` wrapped to `usize::MAX` on over-release.
- `AtomicAllocatorStats::record_deallocation` same fix — `fetch_sub` → `fetch_update` saturating_sub.
- `AtomicAllocatorStats::record_reallocation` captures new `allocated_bytes` from the atomic op return value — old code re-read the field afterward, allowing a stale value to update peak.
- `BatchedStats` no longer uses a shared `thread_local! LOCAL_STATS_BATCH` — that design was cross-instance: a flush triggered by instance B flushed instance A's pending ops into B's global. Now delegates directly to `AtomicAllocatorStats`. `flush()` is a no-op kept for API compatibility.
- `MemorySnapshot::diff` uses `checked_duration_since` → `unwrap_or(Duration::ZERO)` — old `duration_since` panicked in debug builds when snapshots were passed in reversed order.
- `RealTimeMonitor` background thread no longer clears `histogram` on shutdown — old code set `*histogram_arc.write() = None`, destroying data just as callers called `get_histogram()` after `stop()`.
- `MemoryHistogram::add_sample` places underflow samples (below first bucket min) in the first bucket — old code incremented `total_samples` but placed such values in no bucket, diverging percentile denominators.
- `HistogramStats::new` and `update_percentiles` pass `&[50.0, 90.0, 95.0, 99.0]` — old code passed `&[0.5, 0.90, 0.95, 0.99]` which `percentiles()` divided by 100, producing index ≈ 0 for all percentiles.
- `HistogramStats` getter methods (`p50`, `p90`, `p95`, `p99`) compare against `50.0`, `90.0`, `95.0`, `99.0` — updated to match fixed percentile scale.
- `GlobalStats::from_memory_stats` uses `total_allocated_bytes` as `avg_allocation_size` numerator — old code used `current_allocated` which shrinks after deallocations.
- `PredictiveAnalytics::predict_linear` clamps `r_squared.max(0.0)` before computing `confidence_score` — r_squared can be negative for noisy data (ss_res > ss_tot), making confidence negative.
- `MemoryMonitor::determine_action` for `Medium` pressure checks `emergency_threshold` before `warning_threshold` — old code only checked `warning_threshold`, so setting `emergency_threshold = Medium` never produced `PressureAction::Emergency`.
- `MonitoredAllocator::should_allow_allocation` calls `check_pressure` once — old code called `should_allow_large_allocation` (which internally calls `check_pressure`) then `check_pressure` again, doubling OS queries and double-incrementing `pressure_change_count`.
- `pool_get!($pool)` returns `Result` (no longer `.expect("Pool exhausted")`) — use `?` at callsite. Two-arg form `pool_get!(pool, default)` unchanged.

<!-- reviewed: 2026-04-01 (perf: BumpAllocator vtable -> concrete AtomicCursor -21%; PoolAllocator div -> bitmask in deallocate -14%; MetricsExtension split-storage breaking redesign + benchmark suite) -->
<!-- reviewed: 2026-04-02 — clippy cleanup in metrics docs: MemoryMetric references wrapped in backticks -->
