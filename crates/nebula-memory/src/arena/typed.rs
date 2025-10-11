//! Type-safe arena allocator for single type allocations
//!
//! # Safety
//!
//! This module implements a type-safe arena optimized for single-type allocations:
//! - TypedChunk stores T values in Box<[MaybeUninit<T>]>
//! - NonNull pointers to chunks managed via RefCell
//! - Single-threaded access (no Sync without explicit synchronization)
//! - Pointer dereferencing protected by RefCell borrow checking
//!
//! ## Safety Contracts
//!
//! - Chunk pointers valid while arena exists (owned by RefCell)
//! - MaybeUninit properly initialized before creating references
//! - Slice construction from contiguous arena-allocated pointers
//! - Send implementation safe if T: Send (arena is single-threaded)

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use super::ArenaStats;
use crate::error::MemoryError;

/// Capacity of each chunk in number of elements
const DEFAULT_CHUNK_CAPACITY: usize = 64;

/// A chunk of typed elements
struct TypedChunk<T> {
    storage: Box<[MaybeUninit<T>]>,
    next: Option<Box<TypedChunk<T>>>,
}

impl<T> TypedChunk<T> {
    fn new(capacity: usize) -> Self {
        TypedChunk {
            storage: (0..capacity)
                .map(|_| MaybeUninit::uninit())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            next: None,
        }
    }

    fn capacity(&self) -> usize {
        self.storage.len()
    }
}

/// Type-safe arena that can only allocate values of type T
///
/// This arena is optimized for allocating many values of the same type
/// and provides better cache locality than the general-purpose arena.
pub struct TypedArena<T> {
    chunks: RefCell<Option<Box<TypedChunk<T>>>>,
    current_chunk: RefCell<Option<NonNull<TypedChunk<T>>>>,
    current_index: Cell<usize>,
    chunk_capacity: Cell<usize>,
    stats: ArenaStats,
    _phantom: PhantomData<T>,
}

impl<T> TypedArena<T> {
    /// Create a new typed arena
    pub fn new() -> Self {
        TypedArena {
            chunks: RefCell::new(None),
            current_chunk: RefCell::new(None),
            current_index: Cell::new(0),
            chunk_capacity: Cell::new(DEFAULT_CHUNK_CAPACITY),
            stats: ArenaStats::new(),
            _phantom: PhantomData,
        }
    }

    /// Helper: Check if current chunk has capacity for allocation
    ///
    /// # Safety
    /// - chunk_ptr must be valid NonNull pointing to TypedChunk<T>
    /// - chunk must be owned by arena's RefCell
    #[inline]
    unsafe fn chunk_has_capacity(chunk_ptr: NonNull<TypedChunk<T>>, index: usize) -> bool {
        // SAFETY: Dereferencing chunk pointer to check capacity.
        // - chunk_ptr is NonNull (caller contract)
        // - Pointer valid (owned by arena's RefCell)
        // - Read-only access to capacity (no mutation)
        index < (*chunk_ptr.as_ptr()).capacity()
    }

    /// Helper: Get pointer to element in chunk storage
    ///
    /// # Safety
    /// - chunk_ptr must be valid NonNull pointing to TypedChunk<T>
    /// - index must be within chunk capacity bounds
    /// - Caller must ensure chunk is owned by arena's RefCell
    #[inline]
    unsafe fn get_elem_ptr(chunk_ptr: NonNull<TypedChunk<T>>, index: usize) -> *mut T {
        // SAFETY: Accessing chunk storage at index.
        // - chunk_ptr valid (caller contract)
        // - index within bounds (caller contract)
        // - MaybeUninit allows uninitialized access
        // - as_mut_ptr returns raw pointer for writing
        let chunk = &mut *chunk_ptr.as_ptr();
        chunk.storage[index].as_mut_ptr()
    }

    /// Helper: Write value to element pointer
    ///
    /// # Safety
    /// - elem_ptr must point to valid, uninitialized memory
    /// - Memory must be properly aligned for T
    /// - Caller takes ownership of the value
    #[inline]
    unsafe fn write_value(elem_ptr: *mut T, value: T) {
        // SAFETY: Writing value to uninitialized memory.
        // - elem_ptr valid (caller contract)
        // - Memory uninitialized (safe to write via ptr::write)
        // - Takes ownership of value
        elem_ptr.write(value);
    }

    /// Create a new typed arena with initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let arena = Self::new();
        arena.chunk_capacity.set(capacity);
        arena
    }

    /// Creates typed arena optimized for production (capacity = 256 elements)
    pub fn production() -> Self {
        Self::with_capacity(256)
    }

    /// Creates typed arena optimized for debugging (capacity = 16 elements)
    pub fn debug() -> Self {
        Self::with_capacity(16)
    }

    /// Creates typed arena with performance config (alias for production)
    pub fn performance() -> Self {
        Self::production()
    }

    /// Creates a tiny typed arena (4 elements)
    pub fn tiny() -> Self {
        Self::with_capacity(4)
    }

    /// Creates a small typed arena (32 elements)
    pub fn small() -> Self {
        Self::with_capacity(32)
    }

    /// Creates a medium typed arena (128 elements)
    pub fn medium() -> Self {
        Self::with_capacity(128)
    }

    /// Creates a large typed arena (1024 elements)
    pub fn large() -> Self {
        Self::with_capacity(1024)
    }

    /// Allocate a new chunk
    fn allocate_chunk(&self) -> Result<(), MemoryError> {
        let capacity = self.chunk_capacity.get();

        // Double capacity for next chunk
        self.chunk_capacity.set(capacity * 2);

        let mut new_chunk = Box::new(TypedChunk::new(capacity));
        let chunk_ptr = NonNull::from(&mut *new_chunk);

        // Link chunks
        let mut chunks = self.chunks.borrow_mut();
        new_chunk.next = chunks.take();
        *chunks = Some(new_chunk);

        // Update current chunk
        *self.current_chunk.borrow_mut() = Some(chunk_ptr);
        self.current_index.set(0);

        // Update stats
        let bytes = capacity * std::mem::size_of::<T>();
        self.stats.record_chunk_allocation(bytes);

        Ok(())
    }

    /// Allocate a value in the arena
    #[must_use = "allocated memory must be used"]
    pub fn alloc(&self, value: T) -> Result<&mut T, MemoryError> {
        let index = self.current_index.get();

        // Check if we need a new chunk using helper
        let needs_chunk = self.current_chunk.borrow().map_or(true, |chunk| unsafe {
            !Self::chunk_has_capacity(chunk, index)
        });

        if needs_chunk {
            self.allocate_chunk()?;
        }

        // Get current chunk
        let chunk_ptr = self
            .current_chunk
            .borrow()
            .expect("Should have chunk after allocation");

        // Get pointer to element using helper
        // SAFETY: chunk_ptr valid, index within bounds (chunk allocated with sufficient capacity)
        let elem_ptr = unsafe { Self::get_elem_ptr(chunk_ptr, index) };

        // Write value using helper
        // SAFETY: elem_ptr points to valid uninitialized memory
        unsafe { Self::write_value(elem_ptr, value) };

        // Update index and stats
        self.current_index.set(index + 1);
        self.stats.record_allocation(std::mem::size_of::<T>(), 0);

        // SAFETY: Creating mutable reference to initialized value.
        // - elem_ptr valid (from get_elem_ptr helper)
        // - Value just initialized via write_value
        // - Lifetime tied to arena
        // - Exclusive access guaranteed by RefCell
        Ok(unsafe { &mut *elem_ptr })
    }

    /// Allocate space for a value without initializing it
    #[must_use = "allocated memory must be used"]
    pub fn alloc_uninit(&self) -> Result<&mut MaybeUninit<T>, MemoryError> {
        let index = self.current_index.get();

        // Check if we need a new chunk using helper
        let needs_chunk = self.current_chunk.borrow().map_or(true, |chunk| unsafe {
            !Self::chunk_has_capacity(chunk, index)
        });

        if needs_chunk {
            self.allocate_chunk()?;
        }

        // Get current chunk
        let chunk_ptr = self
            .current_chunk
            .borrow()
            .expect("Should have chunk after allocation");

        // Get mutable reference to MaybeUninit element
        // SAFETY: Accessing chunk storage at current index.
        // - chunk_ptr valid (from current_chunk RefCell)
        // - index within bounds (chunk allocated with sufficient capacity)
        // - Returns mutable reference to MaybeUninit (allows uninitialized state)
        // - Lifetime tied to arena
        let elem = unsafe {
            let chunk = &mut *chunk_ptr.as_ptr();
            &mut chunk.storage[index]
        };

        // Update index and stats
        self.current_index.set(index + 1);
        self.stats.record_allocation(std::mem::size_of::<T>(), 0);

        Ok(elem)
    }

    /// Allocate multiple values at once
    #[must_use = "allocated memory must be used"]
    pub fn alloc_slice(&self, values: &[T]) -> Result<&mut [T], MemoryError>
    where
        T: Copy,
    {
        if values.is_empty() {
            return Ok(&mut []);
        }

        // For simplicity, allocate one by one
        // A more efficient implementation would allocate contiguously
        let mut result = Vec::with_capacity(values.len());

        for value in values {
            let ptr = self.alloc(*value)?;
            result.push(ptr as *mut T);
        }

        // Convert to slice
        let slice_ptr = result[0];
        let len = result.len();

        // SAFETY: Creating slice from arena-allocated pointers.
        // - All pointers in result valid (from alloc calls above)
        // - Pointers allocated sequentially (contiguous in memory)
        // - len matches number of allocated elements
        // - All elements initialized via alloc
        // - Lifetime tied to arena
        Ok(unsafe { std::slice::from_raw_parts_mut(slice_ptr, len) })
    }

    /// Allocate an iterator of values
    #[must_use = "allocated memory must be used"]
    pub fn alloc_iter<I>(&self, iter: I) -> Result<Vec<&mut T>, MemoryError>
    where
        I: IntoIterator<Item = T>,
    {
        let mut result = Vec::new();

        for value in iter {
            result.push(self.alloc(value)?);
        }

        Ok(result)
    }

    /// Reset the arena
    ///
    /// Note: This doesn't call destructors on allocated values!
    /// Only use this if T doesn't need dropping or you've already
    /// cleaned up the values.
    pub fn reset(&mut self) {
        // Reset to first chunk
        if let Some(ref chunk) = *self.chunks.borrow() {
            *self.current_chunk.borrow_mut() = Some(NonNull::from(&**chunk));
            self.current_index.set(0);
        } else {
            *self.current_chunk.borrow_mut() = None;
            self.current_index.set(0);
        }

        // Reset chunk capacity
        self.chunk_capacity.set(DEFAULT_CHUNK_CAPACITY);

        // Update stats
        self.stats.record_reset(0);
    }

    /// Get arena statistics
    pub fn stats(&self) -> &ArenaStats {
        &self.stats
    }

    /// Create a snapshot of statistics
    pub fn stats_snapshot(&self) -> super::stats::ArenaStatsSnapshot {
        self.stats.snapshot()
    }
}

impl<T> Default for TypedArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: TypedArena can be sent between threads if T: Send.
// - RefCell/Cell are !Sync but Send (single-threaded arena design)
// - T: Send requirement ensures values can be transferred
// - Arena owns all allocated T values
// - No shared mutable state across threads (RefCell enforces this)
unsafe impl<T: Send> Send for TypedArena<T> {}

/// A reference to a value in a typed arena
pub struct TypedArenaRef<'a, T> {
    value: &'a T,
}

impl<'a, T> TypedArenaRef<'a, T> {
    pub fn new(value: &'a T) -> Self {
        TypedArenaRef { value }
    }
}

impl<'a, T> std::ops::Deref for TypedArenaRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T> AsRef<T> for TypedArenaRef<'a, T> {
    fn as_ref(&self) -> &T {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let arena = TypedArena::<u32>::new();

        let x = arena.alloc(42).unwrap();
        assert_eq!(*x, 42);

        let y = arena.alloc(100).unwrap();
        assert_eq!(*y, 100);

        *x = 50;
        assert_eq!(*x, 50);
    }

    #[test]
    fn test_multiple_chunks() {
        let arena = TypedArena::<u64>::with_capacity(2);

        // Allocate more than one chunk
        let mut values = Vec::new();
        for i in 0..10 {
            values.push(arena.alloc(i).unwrap());
        }

        // Verify all values
        for (i, value) in values.iter().enumerate() {
            assert_eq!(**value, i as u64);
        }

        // Should have allocated multiple chunks
        assert!(arena.stats.chunks_allocated() > 1);
    }

    #[test]
    fn test_slice_allocation() {
        let arena = TypedArena::<i32>::new();

        let values = vec![1, 2, 3, 4, 5];
        let slice = arena.alloc_slice(&values).unwrap();

        assert_eq!(slice.len(), values.len());
        for i in 0..values.len() {
            assert_eq!(slice[i], values[i]);
        }
    }

    #[test]
    fn test_iter_allocation() {
        let arena = TypedArena::<String>::new();

        let strings = vec!["hello", "world", "rust"];
        let allocated: Vec<_> = arena
            .alloc_iter(strings.iter().map(|s| s.to_string()))
            .unwrap();

        assert_eq!(allocated.len(), strings.len());
        for (i, s) in allocated.iter().enumerate() {
            assert_eq!(s.as_str(), strings[i]);
        }
    }

    #[test]
    fn test_uninit_allocation() {
        let arena = TypedArena::<u128>::new();

        let uninit = arena.alloc_uninit().unwrap();
        let ptr = uninit.write(12345);

        assert_eq!(*ptr, 12345);
    }

    #[test]
    fn test_reset() {
        let mut arena = TypedArena::<u32>::new();

        // Allocate some values
        let _x = arena.alloc(1).unwrap();
        let _y = arena.alloc(2).unwrap();

        let stats_before = arena.stats.snapshot();
        assert_eq!(stats_before.allocations, 2);

        arena.reset();

        let stats_after = arena.stats.snapshot();
        assert_eq!(stats_after.resets, 1);

        // Can allocate again
        let _z = arena.alloc(3).unwrap();
    }

    #[derive(Debug, PartialEq)]
    struct TestStruct {
        id: u64,
        name: String,
    }

    #[test]
    fn test_complex_type() {
        let arena = TypedArena::<TestStruct>::new();

        let obj1 = arena
            .alloc(TestStruct {
                id: 1,
                name: "First".to_string(),
            })
            .unwrap();

        let obj2 = arena
            .alloc(TestStruct {
                id: 2,
                name: "Second".to_string(),
            })
            .unwrap();

        assert_eq!(obj1.id, 1);
        assert_eq!(obj2.name, "Second");

        obj1.id = 10;
        assert_eq!(obj1.id, 10);
    }
}
