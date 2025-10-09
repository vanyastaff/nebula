# Error Catalog

Comprehensive guide to all errors in `nebula-memory` with causes, solutions, and examples.

---

## Table of Contents

- [Allocation Errors](#allocation-errors)
  - [ALLOC_OUT_OF_MEMORY](#alloc_out_of_memory)
  - [ALLOC_INVALID_LAYOUT](#alloc_invalid_layout)
  - [ALLOC_INVALID_ALIGNMENT](#alloc_invalid_alignment)
  - [ALLOC_SIZE_OVERFLOW](#alloc_size_overflow)
- [Pool Errors](#pool-errors)
  - [POOL_EXHAUSTED](#pool_exhausted)
  - [POOL_CORRUPTION](#pool_corruption)
- [Stack Errors](#stack-errors)
  - [STACK_LIFO_VIOLATION](#stack_lifo_violation)
  - [STACK_MARKER_INVALID](#stack_marker_invalid)
- [Budget Errors](#budget-errors)
  - [BUDGET_EXCEEDED](#budget_exceeded)
  - [BUDGET_PER_ALLOC_EXCEEDED](#budget_per_alloc_exceeded)

---

## Allocation Errors

### ALLOC_OUT_OF_MEMORY

**Error Code**: `OutOfMemory`

**Cause**: The allocator has exhausted its available memory.

**Common Scenarios**:
1. Allocating more memory than the allocator's total capacity
2. Many small allocations without deallocation (memory fragmentation)
3. Attempting allocation when allocator is full

**Solutions**:

1. **Increase allocator capacity**:
   ```rust
   // Bad - too small
   let allocator = BumpAllocator::new(64)?;
   let data = unsafe { allocator.alloc::<[u8; 1024]>() }?; // ❌ Error!

   // Good - sufficient capacity
   let allocator = BumpAllocator::new(2048)?;
   let data = unsafe { allocator.alloc::<[u8; 1024]>() }?; // ✅ OK
   ```

2. **Reset/reclaim memory** (for bump allocators):
   ```rust
   let allocator = BumpAllocator::new(1024)?;

   // Process batch 1
   for item in batch1 {
       process_with_allocator(&allocator, item)?;
   }

   allocator.reset(); // Free all memory

   // Process batch 2 with clean allocator
   for item in batch2 {
       process_with_allocator(&allocator, item)?;
   }
   ```

3. **Use memory scopes** for automatic cleanup:
   ```rust
   use nebula_memory::memory_scope;

   let allocator = BumpAllocator::new(1024)?;

   for item in items {
       memory_scope!(allocator, {
           // Allocations here are freed after scope
           let temp = unsafe { allocator.alloc::<Buffer>()? };
           process(temp)?;
           Ok(())
       })?;
   }
   ```

4. **Switch to different allocator type**:
   ```rust
   // If you need reuse, use PoolAllocator instead
   let pool = PoolAllocator::new(64, 8, 100)?;

   let ptr = unsafe { pool.alloc::<MyStruct>()? };
   // ... use ...
   unsafe { pool.dealloc(ptr); } // Returns to pool for reuse
   ```

**Error Message Example**:
```
❌ Memory Allocation Error
   Code: OutOfMemory
   Layout: 1024 bytes (alignment: 8)
   Available memory: 256 bytes
   Details: Insufficient memory in allocator

💡 Suggestion:
   Allocator capacity exceeded. Consider:
   1. Increase allocator capacity
   2. Call reset() to reclaim memory
   3. Use a different allocator type
```

---

### ALLOC_INVALID_LAYOUT

**Error Code**: `InvalidLayout`

**Cause**: The requested memory layout is invalid (zero size or invalid alignment).

**Common Scenarios**:
1. Requesting zero-byte allocation
2. Invalid alignment (not power of 2)
3. Size/alignment combination that doesn't make sense

**Solutions**:

1. **Ensure non-zero size**:
   ```rust
   // Bad
   let layout = Layout::from_size_align(0, 8)?; // ❌ Error!

   // Good
   let layout = Layout::from_size_align(64, 8)?; // ✅ OK
   ```

2. **Use TypedAllocator to avoid manual layouts**:
   ```rust
   // Bad - manual layout, error-prone
   let layout = Layout::from_size_align(size, align)?;
   let ptr = allocator.allocate(layout)?;

   // Good - type-safe, automatic layout
   let ptr = unsafe { allocator.alloc::<MyStruct>()? }; // ✅ OK
   ```

3. **Validate input sizes**:
   ```rust
   fn allocate_buffer(allocator: &impl Allocator, size: usize) -> Result<NonNull<u8>> {
       if size == 0 {
           return Err(AllocError::invalid_layout());
       }

       unsafe { allocator.alloc_array::<u8>(size) }
   }
   ```

---

### ALLOC_INVALID_ALIGNMENT

**Error Code**: `InvalidAlignment`

**Cause**: The requested alignment is not a power of 2.

**Common Scenarios**:
1. Using non-power-of-2 alignment values
2. Manually constructing layouts with incorrect alignment

**Solutions**:

1. **Use power-of-2 alignments**:
   ```rust
   // Bad
   let layout = Layout::from_size_align(64, 3)?; // ❌ Error: 3 is not power of 2

   // Good
   let layout = Layout::from_size_align(64, 4)?; // ✅ OK: 4 = 2^2
   ```

2. **Use `next_power_of_two`**:
   ```rust
   use nebula_memory::utils::next_power_of_two;

   let desired_align = 6;
   let actual_align = next_power_of_two(desired_align); // 8
   let layout = Layout::from_size_align(64, actual_align)?; // ✅ OK
   ```

3. **Use type alignments**:
   ```rust
   // Automatically correct alignment
   let align = core::mem::align_of::<MyStruct>();
   let layout = Layout::from_size_align(size, align)?; // ✅ OK
   ```

---

### ALLOC_SIZE_OVERFLOW

**Error Code**: `SizeOverflow`

**Cause**: The requested size causes integer overflow.

**Common Scenarios**:
1. Allocating arrays with extremely large counts
2. Size calculations that overflow `usize`

**Solutions**:

1. **Use checked arithmetic**:
   ```rust
   // Bad
   let total_size = count * item_size; // May overflow!

   // Good
   let total_size = count.checked_mul(item_size)
       .ok_or_else(|| AllocError::size_overflow())?;
   ```

2. **Validate input ranges**:
   ```rust
   fn allocate_array<T>(allocator: &impl Allocator, count: usize) -> Result<NonNull<[T]>> {
       const MAX_ALLOC: usize = 1 << 30; // 1GB limit

       let size = core::mem::size_of::<T>()
           .checked_mul(count)
           .ok_or_else(|| AllocError::size_overflow())?;

       if size > MAX_ALLOC {
           return Err(AllocError::size_overflow());
       }

       unsafe { allocator.alloc_array::<T>(count) }
   }
   ```

---

## Pool Errors

### POOL_EXHAUSTED

**Error Code**: `OutOfMemory` (Pool variant)

**Cause**: All blocks in the pool are currently allocated.

**Solutions**:

1. **Increase pool size**:
   ```rust
   // Bad - too few blocks
   let pool = PoolAllocator::new(64, 8, 10)?;

   // Good - sufficient blocks
   let pool = PoolAllocator::new(64, 8, 100)?;
   ```

2. **Return blocks to pool**:
   ```rust
   let pool = PoolAllocator::new(64, 8, 10)?;
   let mut ptrs = Vec::new();

   // Allocate all blocks
   for _ in 0..10 {
       ptrs.push(unsafe { pool.alloc::<[u8; 64]>()? });
   }

   // Free some blocks
   for ptr in ptrs.drain(..5) {
       unsafe { pool.dealloc(ptr); }
   }

   // Can allocate again
   let ptr = unsafe { pool.alloc::<[u8; 64]>()? }; // ✅ OK
   ```

3. **Use fallback allocator**:
   ```rust
   let pool = PoolAllocator::new(64, 8, 100)?;

   match unsafe { pool.alloc::<MyStruct>() } {
       Ok(ptr) => {
           // Use pool allocation
       }
       Err(_) => {
           // Fallback to Box (system allocator)
           let backup = Box::new(MyStruct::default());
       }
   }
   ```

---

### POOL_CORRUPTION

**Error Code**: `PoolCorruption`

**Cause**: Pool internal structures are corrupted (usually from double-free or use-after-free).

**Prevention**:

1. **Never double-free**:
   ```rust
   let ptr = unsafe { pool.alloc::<MyStruct>()? };
   unsafe { pool.dealloc(ptr); } // ✅ OK
   // unsafe { pool.dealloc(ptr); } // ❌ NEVER DO THIS
   ```

2. **Don't deallocate to wrong pool**:
   ```rust
   let pool1 = PoolAllocator::new(64, 8, 10)?;
   let pool2 = PoolAllocator::new(64, 8, 10)?;

   let ptr = unsafe { pool1.alloc::<MyStruct>()? };
   // unsafe { pool2.dealloc(ptr); } // ❌ WRONG POOL
   unsafe { pool1.dealloc(ptr); } // ✅ CORRECT
   ```

---

## Stack Errors

### STACK_LIFO_VIOLATION

**Error Code**: `InvalidInput` (LIFO violation)

**Cause**: Attempted to deallocate in non-LIFO order.

**Solutions**:

1. **Deallocate in reverse order**:
   ```rust
   let stack = StackAllocator::new(4096)?;

   let ptr1 = unsafe { stack.alloc::<u64>()? };
   let ptr2 = unsafe { stack.alloc::<u64>()? };
   let ptr3 = unsafe { stack.alloc::<u64>()? };

   // Must deallocate in reverse order
   unsafe { stack.dealloc(ptr3); } // ✅ OK
   unsafe { stack.dealloc(ptr2); } // ✅ OK
   unsafe { stack.dealloc(ptr1); } // ✅ OK
   ```

2. **Use markers instead**:
   ```rust
   let stack = StackAllocator::new(4096)?;
   let marker = stack.marker();

   // Allocate many things
   let ptr1 = unsafe { stack.alloc::<u64>()? };
   let ptr2 = unsafe { stack.alloc::<String>()? };
   let ptr3 = unsafe { stack.alloc::<Vec<u8>>()? };

   // Restore to marker - frees all at once
   stack.restore(marker)?; // ✅ OK
   ```

---

## Budget Errors

### BUDGET_EXCEEDED

**Error Code**: `BudgetExceeded`

**Cause**: Total allocated memory exceeds the budget limit.

**Solutions**:

1. **Increase budget**:
   ```rust
   // Bad - too small
   let budget = budget!(1 * 1024 * 1024); // 1MB

   // Good - sufficient budget
   let budget = budget!(100 * 1024 * 1024); // 100MB
   ```

2. **Free memory before allocating**:
   ```rust
   let budget = budget!(10 * 1024 * 1024);

   budget.try_allocate(5 * 1024 * 1024)?; // ✅ OK
   budget.deallocate(5 * 1024 * 1024);    // Free
   budget.try_allocate(5 * 1024 * 1024)?; // ✅ OK again
   ```

---

## Best Practices

### Error Handling Patterns

**1. Graceful Degradation**:
```rust
match unsafe { allocator.alloc::<Buffer>() } {
    Ok(ptr) => use_custom_allocator(ptr),
    Err(e) => {
        eprintln!("Custom allocator failed: {}", e);
        use_system_allocator() // Fallback
    }
}
```

**2. Error Recovery**:
```rust
match unsafe { pool.alloc::<Connection>() } {
    Ok(ptr) => Ok(ptr),
    Err(_) => {
        // Try to reclaim memory
        cleanup_old_connections(&pool);
        // Retry once
        unsafe { pool.alloc::<Connection>() }
    }
}
```

**3. Rich Error Context**:
```rust
unsafe { allocator.alloc::<LargeStruct>() }
    .map_err(|e| {
        eprintln!("Allocation failed: {}", e);
        if let Some(suggestion) = e.suggestion() {
            eprintln!("Tip: {}", suggestion);
        }
        e
    })
```

---

## Getting Help

If you encounter an error not covered here:

1. Check the error message and suggestions
2. Review the [API documentation](https://docs.rs/nebula-memory)
3. See [examples/error_handling.rs](../examples/error_handling.rs) for patterns
4. Open an issue on [GitHub](https://github.com/yourusername/nebula/issues)

---

**Last Updated**: v0.2.0
**See Also**: [CHANGELOG.md](../CHANGELOG.md), [README.md](../README.md)
