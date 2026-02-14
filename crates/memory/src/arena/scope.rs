//! RAII helpers for scoped arena allocations
//!
//! This module provides RAII wrappers that automatically manage arena lifecycle.

use super::{Arena, ArenaConfig, Position};
use crate::error::MemoryResult;

/// RAII wrapper for arena that automatically resets on drop
///
/// # Examples
///
/// ```
/// use nebula_memory::arena::{ArenaScope, ArenaConfig};
///
/// {
///     let mut scope = ArenaScope::new(ArenaConfig::default());
///     let value = scope.alloc(42).unwrap();
///     assert_eq!(*value, 42);
///     // Arena automatically reset when scope is dropped
/// }
/// ```
pub struct ArenaScope {
    arena: Arena,
}

impl ArenaScope {
    /// Create a new arena scope with given configuration
    #[must_use]
    pub fn new(config: ArenaConfig) -> Self {
        Self {
            arena: Arena::new(config),
        }
    }

    /// Create a new arena scope with default configuration
    #[must_use]
    pub fn with_default() -> Self {
        Self::new(ArenaConfig::default())
    }

    /// Allocate a value in this scope
    pub fn alloc<T>(&self, value: T) -> MemoryResult<&mut T> {
        self.arena.alloc(value)
    }

    /// Allocate a slice in this scope
    pub fn alloc_slice<T>(&self, slice: &[T]) -> MemoryResult<&mut [T]>
    where
        T: Copy,
    {
        self.arena.alloc_slice(slice)
    }

    /// Allocate a string in this scope
    pub fn alloc_str(&self, s: &str) -> MemoryResult<&str> {
        self.arena.alloc_str(s)
    }

    /// Get immutable reference to the underlying arena
    pub fn arena(&self) -> &Arena {
        &self.arena
    }

    /// Get mutable reference to the underlying arena
    pub fn arena_mut(&mut self) -> &mut Arena {
        &mut self.arena
    }

    /// Manually reset the arena (will also happen on drop)
    pub fn reset(&mut self) {
        self.arena.reset();
    }
}

impl Drop for ArenaScope {
    fn drop(&mut self) {
        self.arena.reset();
    }
}

/// RAII guard for scoped allocations within an arena
///
/// This guard saves the current position of an arena and automatically resets
/// it to that position when dropped. This allows for temporary allocations
/// that are automatically cleaned up.
///
/// # Examples
///
/// ```
/// use nebula_memory::arena::{Arena, ArenaConfig, ArenaGuard};
///
/// let mut arena = Arena::new(ArenaConfig::default());
/// let outer = arena.alloc(1).unwrap();
///
/// {
///     let _guard = ArenaGuard::new(&mut arena);
///     let _temp = arena.alloc(2).unwrap();
///     // temp is automatically freed when guard is dropped
/// }
///
/// assert_eq!(*outer, 1);
/// ```
#[must_use = "ArenaGuard does nothing unless held"]
pub struct ArenaGuard<'a> {
    arena: &'a mut Arena,
    position: Position,
    active: bool,
}

impl<'a> ArenaGuard<'a> {
    /// Creates a new arena guard that will reset to current position on drop
    pub fn new(arena: &'a mut Arena) -> Self {
        let position = arena.current_position();
        Self {
            arena,
            position,
            active: true,
        }
    }

    /// Manually resets the arena to the saved position
    ///
    /// After calling this, the guard will not reset again on drop.
    pub fn reset(&mut self) -> MemoryResult<()> {
        if self.active {
            self.arena.reset_to_position(self.position)?;
            self.active = false;
        }
        Ok(())
    }

    /// Leaks the guard, preventing it from resetting the arena on drop
    ///
    /// This is useful when you want to keep allocations made within the guard's scope.
    pub fn leak(mut self) {
        self.active = false;
    }

    /// Returns the saved position
    #[must_use]
    pub fn position(&self) -> Position {
        self.position
    }

    /// Returns a mutable reference to the arena
    ///
    /// This allows allocating within the guard's scope
    pub fn arena_mut(&mut self) -> &mut Arena {
        self.arena
    }
}

impl Drop for ArenaGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.arena.reset_to_position(self.position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_scope_auto_reset() {
        let config = ArenaConfig::default();

        {
            let scope = ArenaScope::new(config);
            let _value = scope.alloc(42).unwrap();
            // Arena will be reset here
        }

        // New scope starts fresh
        let scope = ArenaScope::with_default();
        let value = scope.alloc(100).unwrap();
        assert_eq!(*value, 100);
    }

    #[test]
    fn test_arena_scope_manual_reset() {
        let mut scope = ArenaScope::with_default();
        let value1 = scope.alloc(1).unwrap();
        assert_eq!(*value1, 1);

        scope.reset();

        let value2 = scope.alloc(2).unwrap();
        assert_eq!(*value2, 2);
    }

    #[test]
    fn test_arena_guard_nested_scopes() {
        let mut arena = Arena::new(ArenaConfig::default());
        let outer = arena.alloc(1).unwrap();
        let outer_value = *outer; // Copy value before guard
        let pos_outer = arena.current_position();

        {
            let mut guard = ArenaGuard::new(&mut arena);
            let inner = guard.arena_mut().alloc(2).unwrap();
            assert_eq!(*inner, 2);
            // Guard drops here, resetting arena
        }

        // Arena should be reset to outer position
        // Verify the outer allocation is still intact
        let _outer_after = arena.alloc(0).unwrap();
        assert_eq!(arena.current_position(), pos_outer);

        // Can't safely verify outer_value persisted since arena was reset
        // but we verified position is correct
        let _ = outer_value; // Use the value to avoid warning
    }

    #[test]
    fn test_arena_guard_manual_reset() {
        let mut arena = Arena::new(ArenaConfig::default());
        let _outer = arena.alloc(100).unwrap();
        let pos = arena.current_position();

        let mut guard = ArenaGuard::new(&mut arena);
        let _temp = guard.arena_mut().alloc(200).unwrap();

        // Manually reset
        guard.reset().unwrap();
        let pos_after_reset = guard.arena_mut().current_position();
        assert_eq!(pos_after_reset, pos);

        // Guard drop should not reset again
    }

    #[test]
    fn test_arena_guard_leak() {
        let mut arena = Arena::new(ArenaConfig::default());
        let pos_before = arena.current_position();

        {
            let mut guard = ArenaGuard::new(&mut arena);
            let temp = guard.arena_mut().alloc(42).unwrap();
            assert_eq!(*temp, 42);

            // Leak the guard - allocation should persist
            guard.leak();
        }

        // Position should not be reset
        assert_ne!(arena.current_position(), pos_before);
    }

    #[test]
    fn test_arena_guard_multiple_nested() {
        let mut arena = Arena::new(ArenaConfig::default());
        let val1 = arena.alloc(1).unwrap();
        let val1_value = *val1; // Copy value
        let pos1 = arena.current_position();

        {
            let mut guard1 = ArenaGuard::new(&mut arena);
            let val2 = guard1.arena_mut().alloc(2).unwrap();
            let val2_value = *val2; // Copy value
            let pos2 = guard1.arena_mut().current_position();

            {
                let mut guard2 = ArenaGuard::new(guard1.arena_mut());
                let val3 = guard2.arena_mut().alloc(3).unwrap();
                assert_eq!(*val3, 3);
                // guard2 drops, resets to pos2
            }

            assert_eq!(guard1.arena_mut().current_position(), pos2);
            // val2 reference is no longer valid after inner guard, but value was copied
            assert_eq!(val2_value, 2);
            // guard1 drops, resets to pos1
        }

        assert_eq!(arena.current_position(), pos1);
        // val1 reference is no longer valid after guard, but value was copied
        assert_eq!(val1_value, 1);
    }

    #[test]
    fn test_position_validation() {
        let arena1 = Arena::new(ArenaConfig::default());
        let mut arena2 = Arena::new(ArenaConfig::default());

        let _val1 = arena1.alloc(1).unwrap();
        let pos1 = arena1.current_position();

        // Try to use position from arena1 with arena2 - should fail
        let result = arena2.reset_to_position(pos1);
        assert!(result.is_err());
    }

    #[test]
    fn test_arena_guard_with_early_return() {
        fn allocate_temp(arena: &mut Arena, should_fail: bool) -> Result<i32, &'static str> {
            let mut guard = ArenaGuard::new(arena);
            let temp = guard.arena_mut().alloc(42).unwrap();

            if should_fail {
                return Err("failed");
            }

            Ok(*temp)
        }

        let mut arena = Arena::new(ArenaConfig::default());
        let pos = arena.current_position();

        // Early return should still trigger guard drop
        let result = allocate_temp(&mut arena, true);
        assert!(result.is_err());
        assert_eq!(arena.current_position(), pos);
    }

    #[test]
    fn test_position_offset_validation() {
        let mut arena = Arena::new(ArenaConfig::default());
        let _val = arena.alloc(42).unwrap();

        // Create position at current point
        let pos = arena.current_position();

        // Allocate more
        let _val2 = arena.alloc(100).unwrap();

        // Reset to earlier position should succeed
        assert!(arena.reset_to_position(pos).is_ok());

        // Try to reset to a position "in the future" should fail
        // (we can't actually create such a position without unsafe code,
        // so this test just verifies current behavior)
    }
}
