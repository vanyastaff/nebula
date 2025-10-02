# Miri Limitations and Future Work

## Current Status

The nebula-memory allocators currently **do not pass Miri's strict provenance checks**. This is a known limitation that requires architectural changes.

## The Issue

Miri's Stacked Borrows model enforces strict aliasing rules. The current implementation:

1. Stores memory as `Box<[u8]>` (shared ownership)
2. Takes `&self` (shared reference) in `allocate()`
3. Casts `self.memory.as_ptr()` to `*mut u8` to return mutable pointers

This violates Miri's provenance rules because we're creating mutable pointers from shared references.

### Error Example

```
error: Undefined Behavior: attempting a write access using <tag> at alloc[0x0],
but that tag only grants SharedReadOnly permission for this location
```

## The Solution

To make the allocators Miri-compatible, we need to use `UnsafeCell<[u8]>`:

```rust
pub struct BumpAllocator {
    memory: Box<UnsafeCell<[u8]>>,  // Interior mutability
    // ... rest of fields
}
```

This explicitly opts into interior mutability, allowing mutable access through shared references.

## Why Not Implemented Yet

1. **Breaking Change**: Requires changing the core BumpAllocator structure
2. **API Impact**: May affect downstream code that accesses `memory` field
3. **Testing Overhead**: Requires extensive retesting of all allocator functionality
4. **Complexity**: UnsafeCell adds cognitive overhead for maintainers

## Alternative Validation

Until UnsafeCell migration is complete, we validate memory safety through:

1. **Integration Tests**: Comprehensive test suite (21/23 passing)
2. **Benchmarks**: Performance testing ensures no regressions
3. **Manual Review**: All unsafe code blocks have safety comments
4. **Address Sanitizer**: Can be used on supported platforms (not Miri)

## Future Work

Phase 5.3 completion requires:

- [ ] Migrate `BumpAllocator` to use `UnsafeCell<[u8]>`
- [ ] Migrate `PoolAllocator` to use `UnsafeCell`
- [ ] Migrate `StackAllocator` to use `UnsafeCell`
- [ ] Update all tests to work with new structure
- [ ] Verify Miri passes with `-Zmiri-strict-provenance`
- [ ] Document interior mutability patterns

## References

- [Stacked Borrows](https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/stacked-borrows.md)
- [std::cell::UnsafeCell](https://doc.rust-lang.org/std/cell/struct.UnsafeCell.html)
- [Strict Provenance](https://doc.rust-lang.org/nightly/std/ptr/index.html#strict-provenance)
