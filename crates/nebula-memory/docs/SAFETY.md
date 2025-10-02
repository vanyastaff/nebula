# Safety Guarantees

This document describes the safety guarantees provided by nebula-memory allocators and the invariants that must be upheld.

## Core Safety Principles

### 1. Memory Ownership

All allocators follow Rust's ownership model:

- **Allocate**: Returns unique ownership of uninitialized memory
- **Deallocate**: Reclaims ownership and invalidates the pointer
- **No aliasing**: Allocated pointers never overlap (except after deallocation)

### 2. Thread Safety

Allocators are thread-safe when configured with `thread_safe: true`:

- **BumpAllocator**: Uses `AtomicUsize` for cursor operations
- **PoolAllocator**: Uses atomic free list operations
- **StackAllocator**: Uses atomic stack pointer updates

Thread-unsafe variants use `Cell` for better performance in single-threaded contexts.

### 3. Memory Alignment

All allocations respect the requested `Layout` alignment:

```rust
// ✅ Guaranteed to be 16-byte aligned
let layout = Layout::from_size_align(64, 16).unwrap();
let ptr = allocator.allocate(layout).unwrap();
assert_eq!(ptr.as_ptr() as usize % 16, 0);
```

### 4. No Use-After-Free

Allocators prevent use-after-free through:

- **Type safety**: `deallocate()` doesn't return mutable access
- **Reset barriers**: BumpAllocator generation counter detects stale pointers
- **LIFO enforcement**: StackAllocator validates deallocation order

## Allocator-Specific Guarantees

### BumpAllocator

**Guarantees:**
- ✅ Fast O(1) allocation (just pointer bump)
- ✅ No fragmentation (sequential allocation)
- ✅ Bulk O(1) deallocation via `reset()`
- ✅ Thread-safe when configured
- ✅ No double-free (generation counter detection)

**Non-guarantees:**
- ❌ Individual `deallocate()` doesn't reclaim memory
- ❌ Memory cannot be reused until `reset()`
- ❌ Not suitable for long-lived allocations

**Safety invariants:**
```rust
// ✅ Safe usage pattern
let allocator = BumpAllocator::new(4096)?;
{
    let ptr = allocator.allocate(layout)?;
    // ... use ptr ...
}
allocator.reset(); // Bulk deallocation

// ❌ Unsafe: accessing after reset
let ptr = allocator.allocate(layout)?;
allocator.reset();
// UNDEFINED BEHAVIOR: ptr is now invalid!
```

### PoolAllocator

**Guarantees:**
- ✅ O(1) allocation and deallocation
- ✅ Memory reuse (blocks returned to free list)
- ✅ Fixed-size blocks (predictable performance)
- ✅ Thread-safe free list operations
- ✅ No fragmentation within pool

**Non-guarantees:**
- ❌ Pool exhaustion returns error (not panic)
- ❌ Blocks must match pool block_size
- ❌ No automatic capacity growth

**Safety invariants:**
```rust
// ✅ Correct deallocation
let layout = Layout::from_size_align(128, 8)?;
let pool = PoolAllocator::with_config(128, 8, 64, config)?;
let ptr = pool.allocate(layout)?;
pool.deallocate(ptr.cast(), layout); // Same layout!

// ❌ Wrong layout causes UB
let ptr = pool.allocate(Layout::from_size_align(128, 8)?)?;
pool.deallocate(ptr.cast(), Layout::from_size_align(64, 8)?); // WRONG!
```

### StackAllocator

**Guarantees:**
- ✅ LIFO deallocation order enforced
- ✅ Marker-based bulk deallocation
- ✅ Stack overflow detection
- ✅ Nested scope support

**Non-guarantees:**
- ❌ Out-of-order deallocation causes error
- ❌ No automatic reallocation
- ❌ Stack markers must be released in reverse order

**Safety invariants:**
```rust
// ✅ Correct LIFO usage
let stack = StackAllocator::with_config(8192, config)?;
let ptr1 = stack.allocate(layout)?;
let ptr2 = stack.allocate(layout)?;
stack.deallocate(ptr2.cast(), layout); // Last allocated
stack.deallocate(ptr1.cast(), layout); // First allocated

// ❌ Out-of-order deallocation
let ptr1 = stack.allocate(layout)?;
let ptr2 = stack.allocate(layout)?;
stack.deallocate(ptr1.cast(), layout); // ERROR: not last!
```

## Unsafe Code Boundaries

### What is Unsafe

All `allocate()` and `deallocate()` operations are **inherently unsafe** because:

1. **Uninitialized memory**: `allocate()` returns uninitialized bytes
2. **Raw pointers**: Caller must manage pointer lifetime
3. **Manual deallocation**: Caller must call `deallocate()` exactly once
4. **Layout matching**: Same layout must be used for dealloc

### Safety Contracts

When calling `allocator.allocate(layout)`, you **must**:

1. ✅ Initialize memory before reading
2. ✅ Not read/write after `deallocate()`
3. ✅ Call `deallocate()` with same `layout`
4. ✅ Not deallocate twice
5. ✅ Not create aliasing mutable references

When calling `allocator.deallocate(ptr, layout)`, you **must**:

1. ✅ Use pointer from this allocator
2. ✅ Use same `layout` as allocation
3. ✅ Not use pointer after deallocation
4. ✅ Only deallocate each pointer once

### Safe Wrappers

For safe usage, use higher-level abstractions:

```rust
// ✅ Safe: PoolBox ensures deallocation
let pool = PoolAllocator::new(128, 64)?;
let boxed = pool.alloc(42_i32)?; // PoolBox<i32>
// Automatically deallocated on drop

// ✅ Safe: BumpScope ensures reset
let allocator = BumpAllocator::new(4096)?;
{
    let _scope = BumpScope::new(&allocator);
    // Allocations...
} // Automatically reset on drop

// ✅ Safe: StackFrame ensures LIFO
let stack = StackAllocator::new(8192)?;
{
    let _frame = stack.push_frame();
    // Allocations...
} // Automatically popped on drop
```

## Memory Leak Prevention

### Detection Tools

1. **Memory Usage Tracking**:
   ```rust
   assert_eq!(allocator.used_memory(), 0, "Memory leak detected");
   ```

2. **Integration Tests**:
   - `tests/memory_leaks.rs` - 8 tests for leak detection
   - Tests verify `used_memory()` returns to 0

3. **External Tools**:
   - AddressSanitizer (ASan): Detects leaks at runtime
   - Valgrind: Memory leak analysis
   - Miri: (Future) Undefined behavior detection

### Common Leak Patterns

```rust
// ❌ Leak: Forgot to deallocate
let ptr = allocator.allocate(layout)?;
// ... forgot to deallocate!

// ✅ Fixed: Always deallocate
let ptr = allocator.allocate(layout)?;
// ... use ptr ...
allocator.deallocate(ptr.cast(), layout);

// ❌ Leak: Early return
fn process(allocator: &BumpAllocator) -> Result<()> {
    let ptr = allocator.allocate(layout)?;
    if error_condition {
        return Err(error); // Leaked ptr!
    }
    allocator.deallocate(ptr.cast(), layout);
    Ok(())
}

// ✅ Fixed: Use RAII
fn process(allocator: &BumpAllocator) -> Result<()> {
    let _scope = BumpScope::new(allocator);
    let ptr = allocator.allocate(layout)?;
    if error_condition {
        return Err(error); // Scope drops, auto-reset
    }
    Ok(())
} // Auto-reset on drop
```

## Validation Status

| Test Category | Status | Coverage |
|--------------|--------|----------|
| Integration Tests | ✅ 21/23 passing | 91% |
| Memory Leak Tests | ✅ 8/8 passing | 100% |
| Benchmarks | ✅ All passing | - |
| Miri | ⚠️  Blocked | See MIRI_LIMITATIONS.md |

## Future Improvements

### Short Term

- [ ] Add `#[must_use]` to allocation return values
- [ ] Implement Drop guards for all allocators
- [ ] Add compile-time safety checks where possible

### Medium Term

- [ ] Migrate to `UnsafeCell` for Miri compatibility
- [ ] Add optional bounds checking in debug mode
- [ ] Implement allocator versioning for ABI stability

### Long Term

- [ ] Explore const generic safety guarantees
- [ ] Integration with Rust allocator API
- [ ] Formal verification of critical paths

## References

- [Rust Allocator API](https://doc.rust-lang.org/std/alloc/trait.Allocator.html)
- [The Rustonomicon - Unsafe](https://doc.rust-lang.org/nomicon/what-unsafe-does.html)
- [Stacked Borrows](https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md)
