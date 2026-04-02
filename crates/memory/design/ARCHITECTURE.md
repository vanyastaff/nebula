# Architecture

## Problem Statement

- business problem:
  - workflow execution allocates aggressively and needs predictable memory behavior under burst traffic.
- technical problem:
  - provide low-overhead allocation/reuse primitives with safe defaults, observability hooks, and bounded-pressure controls.

## Current Architecture

### Module Map

| Module | Feature flag | Key exported types | Role |
|--------|-------------|-------------------|------|
| `error` | always | `MemoryError` (#[non_exhaustive], 15+ variants), `MemoryResult<T>`, `Result<T>` | Error taxonomy |
| `core` | always | `MemoryConfig`, `MemoryManager`, `MemoryUsage`, `Resettable`, `SyncCell` | Shared contracts and config |
| `allocator` | always | `Allocator`, `TypedAllocator`, `GlobalAllocatorManager`, `AllocError`, `AllocResult`, bump/pool/stack allocators, `MonitoredAllocator`* | Low-level allocation strategies |
| `arena` | `arena` | `Arena`, `TypedArena`, local/thread-safe/cross-thread/scoped arena variants | Scoped fast allocation; bulk reset |
| `pool` | `pool` | `ObjectPool`, `PooledValue`, `ThreadSafePool`, priority/TTL/hierarchical/batch pool variants | Reusable object pools |
| `cache` | `cache` | `ComputeCache`, `CacheConfig`, `CacheKey`, concurrent/partitioned/multi-level/scheduled cache variants; LRU/LFU/FIFO/TTL/random policies | Memoization and multi-level caching |
| `budget` | `budget` | `MemoryBudget`, `BudgetConfig`, `BudgetMetrics`, `BudgetState`, `BudgetPolicy`, `BudgetReservation` | Bounded memory accounting per tenant/workload |
| `monitoring` | `monitoring` | `MemoryMonitor`, `MonitoringConfig`, `IntegratedStats`, `PressureAction`, `MonitoredAllocator` | Pressure detection; system-level reactions |
| `stats` | `stats` | `StatsCollector`, `StatsAggregator`, `MemoryStats`, `Histogram`, `Tracker`, `Snapshot`, `Profiler`, `RealTimeStats` | Usage metrics and profiling |
| `async_support` | `async` | async arena/pool helpers | Tokio-compatible wrappers |
| `syscalls` | `std` | host memory queries | Host memory info |
| `extensions` | `std` | logging/metrics/serialization/async extension traits | Optional integration helpers |
| `utils` | always | `CheckedArithmetic` | Safe arithmetic helpers |

\* `MonitoredAllocator` requires feature `monitoring`

### Data and Control Flow

```
startup:
  nebula_memory::init()
      └─→ GlobalAllocatorManager::init()

workload (caller selects primitive by workload type):
  ├─→ ObjectPool::get() / PooledValue (RAII return on drop)
  │       → PoolExhausted if at capacity (retryable)
  ├─→ Arena::alloc_slice(&data)
  │       → ArenaExhausted if space insufficient (retryable)
  ├─→ ComputeCache::get_or_compute(key, compute_fn)
  │       → CacheMiss if not present (retryable)
  └─→ MemoryBudget::reserve(bytes)
          → BudgetExceeded if over limit (retryable)

monitoring (optional, feature = "monitoring"):
  MemoryMonitor polls syscalls::MemoryInfo
    → emits PressureAction (Normal / Warning / Critical / Emergency)
    → caller/runtime reacts (throttle / shed load / emergency cleanup)

shutdown:
  nebula_memory::shutdown()  → no-op currently; pools/arenas drop via RAII
```

### Known Bottlenecks

- lock contention in shared pool/cache paths under high concurrency
- improperly sized pools or budgets causing avoidable `PoolExhausted`/`BudgetExceeded`

### Key Internal Invariants

- `MemoryError` is `#[non_exhaustive]` — downstream must use `_` match arms
- `PooledValue` returns item to pool on `Drop`; pool is not responsible for item validity after return
- `Arena` and `BumpAllocator` offer only forward allocation; reset is bulk (entire arena), not per-item
- `GlobalAllocatorManager::init()` is idempotent — calling twice is safe
- `Corruption` errors must never be retried; they must escalate to operator
- Feature-gated modules compile out cleanly with no behavioral change to always-on modules
- `SyncCell` provides interior mutability with explicit unsafe boundary — not in public API

## Target Architecture

- target module map:
  - preserve current split, improve policy composition across allocator/pool/cache/budget.
- public contract boundaries:
  - stable: `MemoryError`, `MemoryResult`, `prelude`, key config types, core alloc/pool/cache/budget APIs.
  - evolving: advanced monitoring/profiling/async ergonomics.
- internal invariants:
  - unsafe internals remain encapsulated behind safe public contracts.
  - retryability classification must stay consistent with runtime/resilience expectations.
  - feature off-paths must compile and behave deterministically.

## Design Reasoning

- key trade-off 1:
  - one crate with multiple paradigms reduces integration friction but increases API complexity.
- key trade-off 2:
  - aggressive feature-gating lowers baseline overhead but expands test matrix burden.
- rejected alternatives:
  - single global allocator strategy for all workflows was rejected as too rigid for mixed workloads.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - Temporal/Prefect style explicit operational metrics and pressure-aware behavior.
  - n8n/Activepieces-style practical caching/reuse for execution-heavy paths.
- Reject:
  - Node-RED-like broad mutable global context for memory objects without strict boundaries.
  - hidden automatic policy rewrites under pressure without explicit caller visibility.
- Defer:
  - distributed cross-worker memory coordinator.
  - adaptive auto-tuning of all pool/cache knobs by default.

## Breaking Changes (if any)

- change:
  - likely future convergence toward unified runtime memory config.
- impact:
  - direct per-module configuration bootstrap paths may need migration.
- mitigation:
  - dual API window with explicit shims and migration checklist.

## Open Questions

- Q1: should adaptive pressure actions be centralized in one policy object?
- Q2: which feature set should be considered strict long-term stable baseline?
