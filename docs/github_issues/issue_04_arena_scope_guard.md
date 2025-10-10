---
title: "[HIGH] Implement ArenaGuard for RAII-based arena scope management"
labels: feature, high-priority, nebula-memory
assignees:
milestone: Sprint 5
---

## Problem

ArenaGuard functionality for RAII-based scope management is incomplete. It requires `Arena::current_position()` and `Arena::reset_to_position()` methods which are not yet implemented.

## Current State

- ArenaGuard stub: `crates/nebula-memory/src/arena/scope.rs:79`
- Disabled test: `crates/nebula-memory/src/arena/scope.rs:132`

Missing methods:
- `Arena::current_position() -> Position`
- `Arena::reset_to_position(Position)`

## Impact

🔴 **HIGH Priority** - RAII-based arena scope management is unavailable

**Consequences:**
- Manual arena reset required (error-prone)
- No automatic cleanup on scope exit
- Risk of memory leaks on early returns/panics
- Poor ergonomics for temporary allocations

## Use Case Example

```rust
// Desired API:
fn process_temporary_data(arena: &Arena) -> Result<Output> {
    let _guard = arena.scope_guard();

    // Allocate temporary data
    let temp1 = arena.alloc(data1)?;
    let temp2 = arena.alloc(data2)?;

    // Process...
    let result = compute(temp1, temp2)?;

    // Guard automatically resets arena on drop
    Ok(result)
} // Arena reset here automatically
```

## Action Items

### Implementation
- [ ] Implement `Arena::current_position()` method
  - [ ] Return opaque Position marker
  - [ ] Ensure Position is Copy
  - [ ] Document safety requirements
- [ ] Implement `Arena::reset_to_position(Position)` method
  - [ ] Validate position is from same arena
  - [ ] Handle edge cases (invalid positions)
  - [ ] Update internal state correctly
- [ ] Implement ArenaGuard
  - [ ] Store arena reference and position
  - [ ] Implement Drop trait
  - [ ] Add manual reset() method
  - [ ] Add leak() method to prevent reset

### Testing
- [ ] Re-enable scope guard test
- [ ] Add position validation tests
- [ ] Test nested scopes
- [ ] Test early returns and panics
- [ ] Test invalid position handling
- [ ] Add memory leak tests

### Documentation
- [ ] Document ArenaGuard API
- [ ] Add usage examples
- [ ] Document safety guarantees
- [ ] Note performance characteristics

## Files Affected

```
crates/nebula-memory/src/arena/mod.rs (add methods)
crates/nebula-memory/src/arena/scope.rs (implement ArenaGuard)
```

## Design Considerations

### Position Type
```rust
#[derive(Copy, Clone)]
pub struct Position {
    offset: usize,
    arena_id: u64, // For validation
}
```

### ArenaGuard Implementation
```rust
pub struct ArenaGuard<'a> {
    arena: &'a Arena,
    position: Position,
    active: bool, // For manual reset/leak
}

impl<'a> Drop for ArenaGuard<'a> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.arena.reset_to_position(self.position);
        }
    }
}
```

### Safety Considerations
- Position must be validated against arena ID
- Reset should not invalidate live references (document in safety section)
- Consider using `#[must_use]` on ArenaGuard

## References

- Technical Debt Tracker: [docs/TECHNICAL_DEBT.md](../TECHNICAL_DEBT.md#4-nebula-memory-arena-scope-and-guard)
- Arena allocator documentation

## Acceptance Criteria

- [ ] `current_position()` implemented and tested
- [ ] `reset_to_position()` implemented and tested
- [ ] ArenaGuard fully functional
- [ ] All tests enabled and passing
- [ ] Safety documented thoroughly
- [ ] Examples in documentation
- [ ] No memory leaks in tests
- [ ] Handles panics correctly
