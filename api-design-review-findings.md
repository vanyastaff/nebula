# API Design Review Findings - nebula-memory

**Review Date:** 2026-03-19
**Subtask:** review-7 - API design review
**Scope:** Missing #[must_use], leaky abstractions, inconsistent patterns

---

## Executive Summary

Reviewed the public API surface of nebula-memory for design issues. Found **1 FOOTGUN** affecting 6 RAII guard types missing `#[must_use]` attributes. No leaky abstractions or major inconsistencies detected. Overall API design is clean with consistent patterns.

**Total Findings:** 1 (FOOTGUN)
**Positive Observations:** 5

---

## Finding API-1: Missing #[must_use] on RAII Guard Types [FOOTGUN]

### Severity
**FOOTGUN** - Won't cause crashes, but can lead to silent logic errors

### Description
Six RAII guard/scope types are missing `#[must_use]` attributes, allowing them to be silently dropped without being used. These types exist solely for their Drop implementations, and creating them without using them is almost always a mistake.

### Affected Types

#### 1. `BumpScope<'a>` (allocator/bump/checkpoint.rs:15)
```rust
// CURRENT - Missing #[must_use]
pub struct BumpScope<'a> {
    allocator: &'a BumpAllocator,
    checkpoint: BumpCheckpoint,
}
```

**Impact:** Created scope immediately dropped, checkpoint restoration never happens
```rust
// BUG: Scope dropped immediately, no checkpoint protection
allocator.scope(); // Silently does nothing!

// INTENDED: Scope lives for duration of block
let _scope = allocator.scope();
```

#### 2. `CrossThreadArenaGuard<'a>` (arena/cross_thread.rs:111)
```rust
// CURRENT - Missing #[must_use]
pub struct CrossThreadArenaGuard<'a> {
    guard: parking_lot::MutexGuard<'a, Arena>,
}
```

**Impact:** Lock acquired then immediately released, defeating purpose
```rust
// BUG: Lock acquired and immediately released
arena.lock(); // No protection!

// INTENDED: Lock held for duration
let _guard = arena.lock();
```

#### 3. `PooledValue<'a, T>` (pool/object_pool.rs:380)
```rust
// CURRENT - Missing #[must_use]
pub struct PooledValue<'a, T: Poolable> {
    value: ManuallyDrop<T>,
    pool: &'a ObjectPool<T>,
}
```

**Impact:** Object acquired and immediately returned to pool, wasting allocation
```rust
// BUG: Object returned immediately
pool.get().unwrap(); // Does nothing useful!

// INTENDED: Use the object
let obj = pool.get().unwrap();
```

#### 4. `AsyncPooledValue<T>` (async_support/pool.rs:31)
```rust
// CURRENT - Missing #[must_use]
pub struct AsyncPooledValue<T: Poolable> {
    value: Option<T>,
    return_tx: mpsc::UnboundedSender<T>,
}
```

**Impact:** Async object acquired and immediately returned, async overhead wasted
```rust
// BUG: Await acquisition then immediately drop
pool.acquire().await.unwrap(); // Waste of async work!

// INTENDED: Use the acquired object
let obj = pool.acquire().await.unwrap();
```

#### 5. `Batch<T>` (pool/batch.rs:60)
```rust
// CURRENT - Missing #[must_use]
pub struct Batch<T: Poolable> {
    objects: Vec<T>,
    allocator: *mut BatchAllocator<T>,
}
```

**Impact:** Batch allocated then immediately returned, defeating batch optimization
```rust
// BUG: Batch created and destroyed without use
allocator.allocate_batch(10); // No benefit!

// INTENDED: Use the batch
let batch = allocator.allocate_batch(10);
```

#### 6. `ReservationToken` (budget/reservation.rs:39)
```rust
// CURRENT - Missing #[must_use]
pub struct ReservationToken {
    reservation: Arc<MemoryReservation>,
    released: Mutex<bool>,
}
```

**Impact:** Reservation claimed then immediately released, defeating memory budgeting
```rust
// BUG: Reserve and release immediately
reservation.claim().unwrap(); // Reservation lost!

// INTENDED: Hold reservation
let token = reservation.claim().unwrap();
```

### Root Cause

Inconsistent application of `#[must_use]` across similar types:
- ✅ `ArenaGuard<'a>` HAS `#[must_use]` (arena/scope.rs:106)
- ❌ `CrossThreadArenaGuard<'a>` MISSING
- ❌ `BumpScope<'a>` MISSING

This suggests the pattern is known but not systematically applied.

### Triggering Scenarios

**Scenario 1: Forgotten variable binding**
```rust
// Developer writes:
arena.lock(); // Thinks this "locks the arena"

// Reality: Lock immediately released
```

**Scenario 2: Confusion with non-RAII APIs**
```rust
// Developer familiar with APIs that return () writes:
pool.get().unwrap(); // Expects side effect

// Reality: Object returned to pool immediately
```

**Scenario 3: Refactoring mistake**
```rust
// Before:
let _guard = lock.acquire();

// After (bug introduced):
lock.acquire(); // Underscore removed, now unused
```

### Suggested Fix

Add `#[must_use]` with descriptive messages to all six types:

```rust
// Fix 1: BumpScope
#[must_use = "BumpScope must be held to maintain checkpoint; dropping it restores allocator state"]
pub struct BumpScope<'a> { /* ... */ }

// Fix 2: CrossThreadArenaGuard
#[must_use = "CrossThreadArenaGuard must be held to maintain lock; dropping it releases the lock"]
pub struct CrossThreadArenaGuard<'a> { /* ... */ }

// Fix 3: PooledValue
#[must_use = "PooledValue returns to pool when dropped; use the value before dropping"]
pub struct PooledValue<'a, T: Poolable> { /* ... */ }

// Fix 4: AsyncPooledValue
#[must_use = "AsyncPooledValue returns to pool when dropped; use the value before dropping"]
pub struct AsyncPooledValue<T: Poolable> { /* ... */ }

// Fix 5: Batch
#[must_use = "Batch returns objects to pool when dropped; use the batch before dropping"]
pub struct Batch<T: Poolable> { /* ... */ }

// Fix 6: ReservationToken
#[must_use = "ReservationToken releases reservation when dropped; hold it while using reserved memory"]
pub struct ReservationToken { /* ... */ }
```

### Alternative Fix (More Permissive)

If there are intentional use cases for immediate drops, use shorter messages:
```rust
#[must_use = "guard does nothing unless held"]
```

This matches the pattern used in `ArenaGuard` (scope.rs:106).

### Why This Matters

1. **Silent logic errors:** Code compiles but doesn't work as expected
2. **Performance waste:** Objects allocated then immediately deallocated
3. **Debugging difficulty:** No compiler warning, hard to spot in code review
4. **Consistency:** Other guard types in the crate already use `#[must_use]`

### Testing Strategy

Enable `-D warnings` with `#[must_use]` added, then run:
```bash
cargo clippy --workspace -- -W clippy::must-use-candidate -D warnings
```

Add test cases that intentionally trigger the warning:
```rust
#[test]
#[allow(unused_must_use)]
fn test_guard_without_binding() {
    let arena = CrossThreadArena::new(config);
    arena.lock(); // Should warn with #[must_use]
}
```

---

## Positive Observations

### 1. ✅ Consistent Builder Pattern Usage

All builder methods consistently use `#[must_use = "builder methods must be chained or built"]`:

**Example:** CacheConfig (cache/config.rs:76-123)
```rust
#[must_use = "builder methods must be chained or built"]
pub fn with_policy(mut self, policy: EvictionPolicy) -> Self { /* ... */ }

#[must_use = "builder methods must be chained or built"]
pub fn with_ttl(mut self, ttl: Duration) -> Self { /* ... */ }
```

**Files verified:**
- cache/config.rs
- budget/config.rs
- pool/mod.rs
- All show consistent `#[must_use]` on builder methods

### 2. ✅ No Leaky Abstractions Detected

Public API does not expose internal implementation details:
- No `parking_lot::MutexGuard` in public signatures
- No `std::sync` types in public APIs (only used internally)
- Foundation module exports are intentional and documented
- Internal types remain in `foundation` module, not re-exported in prelude

**Verified:** lib.rs, foundation/mod.rs, public module re-exports

### 3. ✅ Consistent Async Naming Patterns

Async functions follow consistent conventions:
- Timeout variants: `foo()` → `foo_timeout(duration)`
- Try variants: `acquire()` → `try_acquire()`
- No mixing of `async_foo()` vs `foo_async()` patterns

**Example:** AsyncPool (async_support/pool.rs)
```rust
pub async fn acquire(&self) -> MemoryResult<AsyncPooledValue<T>>
pub async fn acquire_timeout(&self, duration: Duration) -> MemoryResult<AsyncPooledValue<T>>
pub async fn try_acquire(&self) -> Option<AsyncPooledValue<T>>
```

**Files verified:**
- async_support/arena.rs (14 async functions)
- async_support/pool.rs (8 async functions)
- cache/simple.rs (9 async functions)

### 4. ✅ Error Type Well-Designed

`MemoryError` follows Rust best practices:
- Marked with `#[must_use = "errors should be handled"]`
- Uses `#[non_exhaustive]` for future extensibility
- Provides `is_retryable()` classifier
- Provides `code()` for structured logging
- Pure constructors (no logging side-effects, per .claude/crates/memory.md)

**Location:** error.rs:13-354

### 5. ✅ Allocation Functions Use #[must_use]

Functions returning allocated memory consistently use `#[must_use = "allocated memory must be used"]`:

**Example:** CrossThreadArenaGuard (cross_thread.rs:117-129)
```rust
#[must_use = "allocated memory must be used"]
pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError>

#[must_use = "allocated memory must be used"]
pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError>
```

---

## Out of Scope (No Issues Found)

### Checked but Clean

1. **Stringly-typed dispatch:** Not found (keys use generic `K: Hash + Eq`)
2. **Missing Send/Sync bounds:** Covered in review-4 (separate finding for ArenaHandle)
3. **Non-chainable builders:** All builders return `Self` correctly
4. **Inconsistent error types:** All use `MemoryResult<T>` consistently
5. **Leaky internal types:** None exposed in public API
6. **Mixed naming conventions:** Naming is consistent across modules

---

## Summary

**Critical Findings:** 0
**Bugs:** 0
**Footguns:** 1 (affecting 6 types)
**Improvements:** 0

**Recommendation:** Add `#[must_use]` to the 6 RAII guard types. This is a low-risk, high-value fix that prevents silent logic errors.

**Priority:** MEDIUM (footgun, not a correctness bug)

**Effort:** LOW (6 one-line additions)

---

## Context Integration

Checked against `.claude/` context files:
- ✅ `.claude/crates/memory.md`: Confirmed error constructors are pure (no logging)
- ✅ `.claude/decisions.md`: No layer violations detected (no upward dependencies)
- ✅ `.claude/pitfalls.md`: No duplicate findings

This finding is NEW and not mentioned in existing context files.
