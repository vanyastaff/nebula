# Data Model: nebula-memory

**Date**: 2026-02-11
**Branch**: `007-memory-prerelease`

This document describes the key entities, their attributes, relationships, and state transitions in the nebula-memory crate. This is a **stabilization** feature — no new entities are introduced.

## Entity Diagram

```
┌─────────────────────────────────────────────┐
│                MemoryConfig                  │
│  (global configuration for all subsystems)   │
├─────────────────────────────────────────────┤
│  allocator: AllocatorConfig                  │
│  pool: PoolConfig                            │
│  arena: ArenaConfig                          │
│  cache: CacheConfig                          │
│  budget: BudgetConfig                        │
│  stats: StatsConfig                          │
└─────────┬───────┬───────┬───────┬───────────┘
          │       │       │       │
          ▼       ▼       ▼       ▼
   ┌──────────┐ ┌─────┐ ┌─────┐ ┌────────┐
   │Allocator │ │Arena│ │Pool │ │  Cache  │
   │(Bump/    │ │     │ │     │ │         │
   │Pool/     │ │     │ │     │ │         │
   │Stack/    │ │     │ │     │ │         │
   │System)   │ │     │ │     │ │         │
   └────┬─────┘ └──┬──┘ └──┬──┘ └────────┘
        │          │       │
        ▼          ▼       ▼
   ┌──────────────────────────┐
   │    MemoryBudget          │
   │  (enforces allocation    │
   │   limits per scope)      │
   ├──────────────────────────┤
   │  capacity: usize         │
   │  used: AtomicUsize       │
   │  children: Vec<Budget>   │
   └──────────┬───────────────┘
              │ parent-child
              ▼
   ┌──────────────────────────┐
   │    MemoryBudget (child)  │
   │  bounded by parent       │
   └──────────────────────────┘

   ┌──────────────────────────┐
   │    MemoryMonitor         │
   │  (system pressure)       │
   ├──────────────────────────┤
   │  config: MonitoringConfig│
   │  pressure: PressureLevel │
   │  actions: PressureAction │
   └──────────────────────────┘

   ┌──────────────────────────┐
   │    AllocatorStats        │
   │  (per-allocator metrics) │
   ├──────────────────────────┤
   │  allocations: u64        │
   │  deallocations: u64      │
   │  bytes_allocated: u64    │
   │  bytes_deallocated: u64  │
   │  peak_usage: u64         │
   └──────────────────────────┘
```

## Entities

### Allocator (trait: `allocator::traits::Allocator`)

The core memory provider. All allocators implement the `Allocator` trait.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| capacity | `usize` | Total available memory |
| used | `usize` | Currently allocated bytes |
| thread_safe | `bool` | Whether concurrent use is safe |

**Implementations**: BumpAllocator, PoolAllocator, StackAllocator, SystemAllocator

**State transitions**: Created → Active (allocating/deallocating) → Reset/Dropped

### Arena

Region-scoped allocator that deallocates all memory at once on drop.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| config | `ArenaConfig` | Growth strategy, capacity, hints |
| chunks | `Vec<Chunk>` | Allocated memory chunks |
| current_offset | `usize` | Current position in active chunk |

**Variants**: Arena, ThreadSafeArena, TypedArena<T>, LocalArena, CrossThreadArena, StreamingArena

**State transitions**: Created → Allocating → Dropped (all memory freed)

### MemoryBudget

Capacity constraint for a group of allocations. Supports parent-child hierarchy.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| capacity | `usize` | Maximum allowed bytes |
| used | `AtomicUsize` | Currently consumed bytes |
| parent | `Option<Arc<MemoryBudget>>` | Parent budget (if child) |
| children | `Vec<Arc<MemoryBudget>>` | Child budgets |

**State transitions**: Created → Active → Exhausted (rejects new allocations) → Released (capacity returned to parent)

### ObjectPool

Pre-allocated fixed-size block collection for object reuse.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| block_size | `usize` | Size of each block |
| capacity | `usize` | Maximum number of blocks |
| free_list | collection | Available blocks |
| config | `PoolConfig` | Growth, shrink, TTL settings |

**Variants**: ObjectPool, ThreadSafePool, LockFreePool, TtlPool, PriorityPool, HierarchicalPool

**State transitions**: Created → Active (lending/returning blocks) → Exhausted (if no growth) → Dropped

### Cache

Computation result storage with eviction policies.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| capacity | `usize` | Maximum entries |
| policy | `EvictionPolicy` | LRU/LFU/FIFO/ARC |
| ttl | `Option<Duration>` | Time-to-live per entry |

**Variants**: ComputeCache, ConcurrentComputeCache, MultiLevelCache, PartitionedCache, ScheduledCache, AsyncCache

### AllocatorStats

Per-allocator metrics tracking.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| allocations | `AtomicU64` | Total allocation count |
| deallocations | `AtomicU64` | Total deallocation count |
| bytes_allocated | `AtomicU64` | Total bytes allocated |
| bytes_deallocated | `AtomicU64` | Total bytes deallocated |
| peak_usage | `AtomicU64` | Peak concurrent usage |

### MemoryMonitor

System-level memory pressure observer.

| Attribute | Type | Description |
| --------- | ---- | ----------- |
| config | `MonitoringConfig` | Thresholds, check intervals |
| current_pressure | `PressureLevel` | Current system pressure |
| last_check | `Instant` | Last pressure check time |

**State transitions**: Idle → Monitoring → PressureDetected → ActionTriggered → Idle

## Relationships

1. **MemoryConfig** contains configuration for all subsystems (1:1 with each config type)
2. **MemoryBudget** has parent-child hierarchy (1:N self-referential)
3. **Allocator** may have **AllocatorStats** (optional, via `track_stats` config)
4. **Arena** uses an **Allocator** internally for chunk allocation
5. **MemoryMonitor** observes system state independently of allocators
6. **AllocatorManager** (global) tracks all registered **Allocator** instances

## Validation Rules

- Budget capacity must be > 0
- Child budget capacity must be <= parent remaining capacity
- Pool block_size must be > 0 and properly aligned
- Arena chunk size must be >= minimum allocation size
- Cache capacity must be > 0
- All configuration values validated at construction time (builder pattern with `.validate()`)
