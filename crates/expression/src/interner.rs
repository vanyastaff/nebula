//! String interning for identifiers and literals
//!
//! This module provides string interning to reduce memory allocations and
//! enable fast string comparisons via pointer equality.
//!
//! # Cost model
//!
//! - `intern` is `O(1)` amortised on hit and `O(1)` allocate-and-insert on miss. The fast path
//!   takes a read lock; the slow path takes a write lock and double-checks for races.
//! - `Clone` deep-copies the entire `HashSet<Arc<str>>` — `O(n)` in the number of unique strings.
//!   The contained `Arc<str>` values are still `O(1)` to clone individually, but the `HashSet`
//!   itself reallocates. Avoid cloning interners on hot paths; share via `Arc<StringInterner>`
//!   instead.
//! - `Default::default()` is equivalent to `StringInterner::new()` — empty, ready to use.

use std::{collections::HashSet, sync::Arc};

use parking_lot::RwLock;

/// A thread-safe string interner
///
/// Deduplicates strings by maintaining a single copy of each unique string.
/// Returns `Arc<str>` for cheap cloning and comparison.
#[derive(Debug, Default)]
pub struct StringInterner {
    strings: RwLock<HashSet<Arc<str>>>,
}

impl StringInterner {
    /// Create a new empty interner
    pub fn new() -> Self {
        Self {
            strings: RwLock::new(HashSet::new()),
        }
    }

    /// Create a new interner with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: RwLock::new(HashSet::with_capacity(capacity)),
        }
    }

    /// Intern a string, returning a shared reference
    ///
    /// If the string is already interned, returns the existing Arc.
    /// Otherwise, creates a new Arc and stores it.
    pub fn intern(&self, s: &str) -> Arc<str> {
        // Fast path: check if already interned (read lock)
        {
            let strings = self.strings.read();
            if let Some(arc) = strings.get(s) {
                return Arc::clone(arc);
            }
        }

        // Slow path: intern new string (write lock)
        let mut strings = self.strings.write();

        // Double-check in case another thread interned it
        if let Some(arc) = strings.get(s) {
            return Arc::clone(arc);
        }

        // Create new Arc and insert
        let arc: Arc<str> = Arc::from(s);
        strings.insert(Arc::clone(&arc));
        arc
    }

    /// Get the number of unique strings interned
    pub fn len(&self) -> usize {
        self.strings.read().len()
    }

    /// Check if the interner is empty
    pub fn is_empty(&self) -> bool {
        self.strings.read().is_empty()
    }

    /// Clear all interned strings
    pub fn clear(&self) {
        self.strings.write().clear();
    }
}

impl Clone for StringInterner {
    fn clone(&self) -> Self {
        Self {
            strings: RwLock::new(self.strings.read().clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_interning() {
        let interner = StringInterner::new();

        let s1 = interner.intern("hello");
        let s2 = interner.intern("hello");
        let s3 = interner.intern("world");

        // Same string should return same Arc (pointer equality)
        assert!(Arc::ptr_eq(&s1, &s2));

        // Different strings should have different Arcs
        assert!(!Arc::ptr_eq(&s1, &s3));

        assert_eq!(interner.len(), 2);
    }

    #[test]
    fn test_clear() {
        let interner = StringInterner::new();

        interner.intern("test");
        assert_eq!(interner.len(), 1);

        interner.clear();
        assert_eq!(interner.len(), 0);
    }

    #[test]
    fn test_with_capacity() {
        let interner = StringInterner::with_capacity(100);
        assert_eq!(interner.len(), 0);

        for i in 0..10 {
            interner.intern(&format!("str{i}"));
        }

        assert_eq!(interner.len(), 10);
    }

    #[test]
    fn default_constructs_empty_interner() {
        let interner = StringInterner::default();
        assert_eq!(interner.len(), 0);
        assert!(interner.is_empty());

        // And it's actually usable, not just empty.
        let s = interner.intern("hello");
        assert_eq!(&*s, "hello");
        assert_eq!(interner.len(), 1);
    }
}
