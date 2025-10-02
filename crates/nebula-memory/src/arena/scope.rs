//! RAII helpers for scoped arena allocations
//!
//! This module provides RAII wrappers that automatically manage arena lifecycle.

use super::{Arena, ArenaConfig};
use crate::core::error::MemoryResult;

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
    pub fn new(config: ArenaConfig) -> Self {
        Self {
            arena: Arena::new(config),
        }
    }

    /// Create a new arena scope with default configuration
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

// TODO: ArenaGuard - requires Arena::current_position() and Arena::reset_to_position()
//
// /// RAII guard for scoped allocations within an arena
// pub struct ArenaGuard<'a> {
//     arena: &'a mut Arena,
//     saved_position: usize,
// }
//
// impl<'a> ArenaGuard<'a> {
//     pub fn new(arena: &'a mut Arena) -> Self {
//         let saved_position = arena.current_position();
//         Self { arena, saved_position }
//     }
// }
//
// impl<'a> Drop for ArenaGuard<'a> {
//     fn drop(&mut self) {
//         self.arena.reset_to_position(self.saved_position);
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_scope_auto_reset() {
        let config = ArenaConfig::default();

        {
            let mut scope = ArenaScope::new(config);
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

    // TODO: Re-enable when ArenaGuard is implemented
    // #[test]
    // fn test_arena_guard_nested_scopes() {
    //     let mut arena = Arena::new(ArenaConfig::default());
    //     let outer = arena.alloc(1).unwrap();
    //     {
    //         let _guard = ArenaGuard::new(&mut arena);
    //         let inner = arena.alloc(2).unwrap();
    //         assert_eq!(*inner, 2);
    //     }
    //     assert_eq!(*outer, 1);
    // }
}
