# Trait API Contracts: nebula-memory

**Date**: 2026-02-11
**Branch**: `007-memory-prerelease`

This document specifies the public trait APIs that constitute the contract surface of nebula-memory. These are existing traits being stabilized — no new traits are introduced.

## Core Allocator Trait

```rust
/// Core allocator interface for all memory providers.
///
/// # Safety
/// Implementations must ensure:
/// - Returned pointers are valid and properly aligned
/// - Deallocate is only called with pointers from this allocator
/// - Grow/shrink preserve existing data up to min(old_size, new_size)
pub unsafe trait Allocator: Send {
    fn allocate(&self, layout: Layout) -> AllocResult<NonNull<u8>>;
    fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);
    fn reallocate(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> AllocResult<NonNull<u8>>;
    fn grow(&self, ptr: NonNull<u8>, old: Layout, new_size: usize) -> AllocResult<NonNull<u8>>;
    fn shrink(&self, ptr: NonNull<u8>, old: Layout, new_size: usize) -> AllocResult<NonNull<u8>>;
}
```

**Contract**: All methods return `Result<_, MemoryError>`. Never panics.

## TypedAllocator Trait (blanket impl)

```rust
/// Type-safe allocation operations. Blanket-implemented for all Allocator types.
pub trait TypedAllocator: Allocator {
    unsafe fn alloc<T>(&self) -> AllocResult<NonNull<T>>;
    unsafe fn alloc_init<T>(&self, value: T) -> AllocResult<NonNull<T>>;
    unsafe fn alloc_array<T>(&self, count: usize) -> AllocResult<NonNull<T>>;
    unsafe fn dealloc_typed<T>(&self, ptr: NonNull<T>);
    unsafe fn dealloc_array<T>(&self, ptr: NonNull<T>, count: usize);
}
```

**Contract**: Layout is computed from `T` automatically. Prevents layout construction errors.

## MemoryUsage Trait

```rust
/// Track memory consumption of an allocator.
pub trait MemoryUsage {
    fn used_memory(&self) -> usize;
    fn available_memory(&self) -> Option<usize>;
    fn total_capacity(&self) -> usize;
}
```

**Contract**: Values are consistent snapshots (not necessarily atomic across all three).

## StatisticsProvider Trait

```rust
/// Optionally provide allocation statistics.
pub trait StatisticsProvider {
    fn statistics(&self) -> Option<AllocatorStats>;
}
```

**Contract**: Returns `None` when stats tracking is disabled. Never panics.

## Resettable Trait

```rust
/// Reset allocator to initial state, freeing all allocations.
pub trait Resettable {
    fn reset(&self);
}
```

**Contract**: After reset, capacity is fully available. All prior pointers are invalidated.

## ArenaAllocate Trait

```rust
/// Arena-specific allocation interface.
pub trait ArenaAllocate {
    fn arena_alloc(&self, layout: Layout) -> AllocResult<NonNull<u8>>;
    fn arena_alloc_typed<T>(&self, value: T) -> AllocResult<&T>;
    fn remaining_capacity(&self) -> usize;
    fn reset(&mut self);
}
```

**Contract**: Arena allocations are freed in bulk via `reset()` or drop. Individual deallocation is a no-op.

## Poolable Trait

```rust
/// Objects that can be managed by an ObjectPool.
pub trait Poolable: Send {
    fn reset(&mut self);
}
```

**Contract**: `reset()` must restore the object to a clean reusable state.

## Error Contract

```rust
/// All fallible operations return Result<T, MemoryError>.
pub type AllocResult<T> = Result<T, MemoryError>;

/// Error type with actionable context. Never panics.
#[derive(Debug, Error)]
pub enum MemoryError {
    AllocationFailed { size: usize, align: usize },
    OutOfMemory { requested: usize, available: usize },
    PoolExhausted { capacity: usize },
    BudgetExceeded { requested: usize, remaining: usize },
    InvalidLayout { reason: String },
    // ... additional variants
}
```

**Contract**: Every error variant includes enough context for diagnosis and recovery. No `panic!()` in any production code path.

## Post-Cleanup Feature Flags

After pre-release cleanup, the feature flags will be:

| Feature | Default | Description |
| ------- | ------- | ----------- |
| `std` | yes | Standard library (now required) |
| `pool` | yes | Object pool allocators |
| `arena` | yes | Arena allocators |
| `cache` | yes | Computation caching |
| `budget` | yes | Memory budgeting |
| `logging` | yes | nebula-log integration |
| `stats` | no | Statistics tracking |
| `monitoring` | no | Memory pressure monitoring |
| `profiling` | no | Detailed profiling |
| `adaptive` | no | Adaptive allocation |
| `async` | no | Async allocator support |
| `simd` | no | SIMD memory operations |
| `numa-aware` | no | NUMA awareness (experimental) |
| `linux-optimizations` | no | Linux-specific optimizations |
| `backtrace-feature` | no | Backtrace capture |
| `nightly` | no | Nightly-only features |
| `full` | no | All non-experimental features |

**Removed**: `compression`, `alloc`, `streaming`
