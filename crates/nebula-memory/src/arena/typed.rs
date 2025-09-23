//! Type-safe arena allocator for single type allocations

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

    /// Create a new typed arena with initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let mut arena = Self::new();
        arena.chunk_capacity.set(capacity);
        arena
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
    pub fn alloc(&self, value: T) -> Result<&mut T, MemoryError> {
        let index = self.current_index.get();

        // Check if we need a new chunk
        let needs_chunk = self
            .current_chunk
            .borrow()
            .map_or(true, |chunk| unsafe { index >= (*chunk.as_ptr()).capacity() });

        if needs_chunk {
            self.allocate_chunk()?;
        }

        // Get current chunk
        let chunk_ptr = self.current_chunk.borrow().expect("Should have chunk after allocation");

        // Get pointer to element
        let elem_ptr = unsafe {
            let chunk = &mut *chunk_ptr.as_ptr();
            chunk.storage[self.current_index.get()].as_mut_ptr()
        };

        // Write value
        unsafe {
            elem_ptr.write(value);
        }

        // Update index
        self.current_index.set(index + 1);

        // Update stats
        self.stats.record_allocation(std::mem::size_of::<T>(), 0);

        Ok(unsafe { &mut *elem_ptr })
    }

    /// Allocate space for a value without initializing it
    pub fn alloc_uninit(&self) -> Result<&mut MaybeUninit<T>, MemoryError> {
        let index = self.current_index.get();

        // Check if we need a new chunk
        let needs_chunk = self
            .current_chunk
            .borrow()
            .map_or(true, |chunk| unsafe { index >= (*chunk.as_ptr()).capacity() });

        if needs_chunk {
            self.allocate_chunk()?;
        }

        // Get current chunk
        let chunk_ptr = self.current_chunk.borrow().expect("Should have chunk after allocation");

        // Get pointer to element
        let elem_ptr = unsafe {
            let chunk = &mut *chunk_ptr.as_ptr();
            &mut chunk.storage[self.current_index.get()]
        };

        // Update index
        self.current_index.set(index + 1);

        // Update stats
        self.stats.record_allocation(std::mem::size_of::<T>(), 0);

        Ok(elem_ptr)
    }

    /// Allocate multiple values at once
    pub fn alloc_slice(&self, values: &[T]) -> Result<&mut [T], MemoryError>
    where T: Copy {
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

        Ok(unsafe { std::slice::from_raw_parts_mut(slice_ptr, len) })
    }

    /// Allocate an iterator of values
    pub fn alloc_iter<I>(&self, iter: I) -> Result<Vec<&mut T>, MemoryError>
    where I: IntoIterator<Item = T> {
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
        let allocated: Vec<_> = arena.alloc_iter(strings.iter().map(|s| s.to_string())).unwrap();

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

        let obj1 = arena.alloc(TestStruct { id: 1, name: "First".to_string() }).unwrap();

        let obj2 = arena.alloc(TestStruct { id: 2, name: "Second".to_string() }).unwrap();

        assert_eq!(obj1.id, 1);
        assert_eq!(obj2.name, "Second");

        obj1.id = 10;
        assert_eq!(obj1.id, 10);
    }
}
