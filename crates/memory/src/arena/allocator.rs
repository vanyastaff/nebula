//! Arena-based allocator implementation
//!
//! This module provides the [`ArenaAllocator`] type, which can be used to
//! allocate memory from an arena. It also provides the [`ArenaBackedVec`] type
//! for arena-allocated vectors.
//!
//! # Safety
//!
//! This module contains arena-based allocation utilities:
//! - `ArenaAllocator`: Wrapper providing allocator interface
//! - `ArenaBackedVec`: Arena-allocated vector with manual growth
//!
//! ## Safety Contracts
//!
//! - `allocate/allocate_slice`: Caller must initialize before use
//! - `ArenaBackedVec`: Internal pointer valid while arena alive
//! - push: Bounds-checked, writes to allocated capacity
//! - pop/get: Pointer arithmetic within allocated range
//! - clear: `drop_in_place` for all elements
//! - `as_slice`: Creates slice from valid pointer and len
//!
//! ## Memory Management
//!
//! - Arc<Arena>: Shared ownership of arena
//! - `ArenaBackedVec`: Doesn't deallocate (arena manages memory)
//! - Drop: Calls clear but doesn't free memory (arena-owned)

use std::alloc::Layout;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::Arc;

use super::{Arena, ArenaAllocate, ThreadSafeArena};
use crate::error::MemoryError;

/// An arena-backed memory allocator
///
/// This allows manual allocation through arenas with a familiar allocator
/// interface.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nebula_memory::arena::{Arena, ArenaAllocator};
///
/// let arena = Arc::new(Arena::new(Default::default()));
/// let allocator = ArenaAllocator::new(arena);
///
/// // Allocate memory
/// let layout = std::alloc::Layout::from_size_align(64, 8).unwrap();
/// let ptr = unsafe { allocator.allocate(layout).unwrap() };
/// ```
pub struct ArenaAllocator<A> {
    arena: Arc<A>,
}

impl<A> ArenaAllocator<A> {
    /// Create a new arena allocator
    pub fn new(arena: Arc<A>) -> Self {
        ArenaAllocator { arena }
    }

    /// Get a reference to the underlying arena
    #[must_use]
    pub fn arena(&self) -> &Arc<A> {
        &self.arena
    }
}

impl<A: ArenaAllocate> ArenaAllocator<A> {
    /// Allocate memory with the given layout
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is properly initialized before use
    pub unsafe fn allocate(&self, layout: Layout) -> Result<NonNull<u8>, MemoryError> {
        // SAFETY: Allocating from arena.
        // - Forwarding to arena.alloc_bytes with layout parameters
        // - arena.alloc_bytes returns null on failure (handled by NonNull::new)
        // - Caller must initialize memory before use (documented contract)
        unsafe {
            let ptr = self.arena.alloc_bytes(layout.size(), layout.align())?;
            NonNull::new(ptr)
                .ok_or_else(|| MemoryError::allocation_failed(layout.size(), layout.align()))
        }
    }

    /// Allocate a slice of memory with the given layout
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is properly initialized before use
    pub unsafe fn allocate_slice(&self, layout: Layout) -> Result<NonNull<[u8]>, MemoryError> {
        // SAFETY: Allocating slice from arena.
        // - arena.alloc_bytes allocates layout.size() bytes with proper alignment
        // - from_raw_parts_mut creates slice from valid pointer and size
        // - ptr is valid for layout.size() bytes (arena guarantees)
        // - Caller must initialize memory before use (documented contract)
        // - NonNull::from converts &mut [u8] to NonNull<[u8]>
        unsafe {
            let ptr = self.arena.alloc_bytes(layout.size(), layout.align())?;

            // Create a slice from the allocated memory
            let slice = std::slice::from_raw_parts_mut(ptr, layout.size());

            Ok(NonNull::from(slice))
        }
    }

    /// Allocate and initialize a value
    #[must_use = "allocated memory must be used"]
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        self.arena.alloc(value)
    }

    /// Allocate and initialize a slice
    #[must_use = "allocated memory must be used"]
    pub fn alloc_slice_copy<T: Copy>(&self, slice: &[T]) -> Result<&mut [T], MemoryError> {
        self.arena.alloc_slice(slice)
    }

    /// Create a new vector backed by this arena
    #[must_use]
    pub fn new_vec<T>(&self) -> ArenaBackedVec<T, A> {
        ArenaBackedVec::new(self.arena.clone())
    }

    /// Create a new vector with capacity backed by this arena
    pub fn new_vec_with_capacity<T>(
        &self,
        capacity: usize,
    ) -> Result<ArenaBackedVec<T, A>, MemoryError> {
        ArenaBackedVec::with_capacity(capacity, self.arena.clone())
    }
}

impl ArenaAllocator<Arena> {
    /// Create a new allocator with a basic arena
    #[must_use]
    pub fn new_basic() -> Self {
        Self::new(Arc::new(Arena::new(Default::default())))
    }

    /// Create a new allocator with a basic arena with specified capacity
    #[must_use]
    pub fn new_basic_with_capacity(capacity: usize) -> Self {
        Self::new(Arc::new(Arena::with_capacity(capacity)))
    }
}

impl ArenaAllocator<ThreadSafeArena> {
    /// Create a new allocator with a thread-safe arena
    #[must_use]
    pub fn new_thread_safe() -> Self {
        Self::new(Arc::new(ThreadSafeArena::new(Default::default())))
    }

    /// Create a new allocator with a thread-safe arena with specified config
    #[must_use]
    pub fn new_thread_safe_with_config(config: super::ArenaConfig) -> Self {
        Self::new(Arc::new(ThreadSafeArena::new(config)))
    }
}

impl<A> Clone for ArenaAllocator<A> {
    fn clone(&self) -> Self {
        ArenaAllocator {
            arena: Arc::clone(&self.arena),
        }
    }
}

/// A vector that uses arena allocation
///
/// This is a simplified vector implementation that allocates its storage
/// from an arena. It does not support automatic growth or shrinking.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nebula_memory::arena::{Arena, ArenaBackedVec};
///
/// let arena = Arc::new(Arena::new(Default::default()));
/// let mut vec = ArenaBackedVec::with_capacity(10, arena).unwrap();
///
/// vec.push(1).unwrap();
/// vec.push(2).unwrap();
/// vec.push(3).unwrap();
///
/// assert_eq!(vec.len(), 3);
/// assert_eq!(vec[0], 1);
/// ```
pub struct ArenaBackedVec<T, A: ArenaAllocate> {
    data: *mut T,
    len: usize,
    capacity: usize,
    allocator: Arc<A>,
    _marker: PhantomData<T>,
}

impl<T, A: ArenaAllocate> ArenaBackedVec<T, A> {
    /// Create a new empty vector
    pub fn new(allocator: Arc<A>) -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
            capacity: 0,
            allocator,
            _marker: PhantomData,
        }
    }

    /// Create a new vector with the given capacity
    #[must_use = "allocated vector must be used"]
    pub fn with_capacity(capacity: usize, allocator: Arc<A>) -> Result<Self, MemoryError> {
        if capacity == 0 {
            return Ok(Self::new(allocator));
        }

        let layout = Layout::array::<T>(capacity)
            .map_err(|_| MemoryError::invalid_layout("array layout error"))?;

        // SAFETY: Allocating capacity from arena.
        // - allocator.alloc_bytes allocates layout.size() bytes
        // - Cast to *mut T safe (proper size and alignment from Layout::array)
        let ptr = unsafe { allocator.alloc_bytes(layout.size(), layout.align())? };

        Ok(Self {
            data: ptr.cast::<T>(),
            len: 0,
            capacity,
            allocator,
            _marker: PhantomData,
        })
    }

    /// Push a value onto the vector
    #[must_use = "operation result must be checked"]
    pub fn push(&mut self, value: T) -> Result<(), MemoryError> {
        if self.len >= self.capacity {
            return Err(MemoryError::allocation_too_large(
                self.len + 1,
                self.capacity,
            ));
        }

        // SAFETY: Writing value to allocated slot.
        // - data.add(len) is within capacity (checked above)
        // - Slot is uninitialized or previously popped (safe to write)
        // - len incremented after write (maintains invariant)
        unsafe {
            self.data.add(self.len).write(value);
        }

        self.len += 1;
        Ok(())
    }

    /// Pop a value from the vector
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }

        self.len -= 1;
        // SAFETY: Reading last element.
        // - len decremented before read
        // - data.add(len) was valid element (len > 0 before decrement)
        // - read() moves value out (slot becomes uninitialized)
        unsafe { Some(self.data.add(self.len).read()) }
    }

    /// Get a reference to an element
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len {
            // SAFETY: Creating reference to element.
            // - index < len (checked above)
            // - data.add(index) points to valid initialized element
            // - Reference lifetime bound to &self
            unsafe { Some(&*self.data.add(index)) }
        } else {
            None
        }
    }

    /// Get a mutable reference to an element
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            // SAFETY: Creating mutable reference to element.
            // - index < len (checked above)
            // - data.add(index) points to valid initialized element
            // - &mut self ensures exclusive access
            // - Reference lifetime bound to &mut self
            unsafe { Some(&mut *self.data.add(index)) }
        } else {
            None
        }
    }

    /// Returns the length of the vector
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the vector is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the capacity of the vector
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear the vector
    pub fn clear(&mut self) {
        // SAFETY: Dropping all elements.
        // - Loop iterates from 0 to len
        // - Each data.add(i) points to valid initialized element
        // - drop_in_place runs destructor
        // - len set to 0 after loop (elements become uninitialized)
        unsafe {
            // Drop all elements
            for i in 0..self.len {
                std::ptr::drop_in_place(self.data.add(i));
            }
        }
        self.len = 0;
    }

    /// Get a slice of the vector
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: Creating slice from vector data.
        // - data is valid pointer (allocated from arena)
        // - len represents number of initialized elements
        // - from_raw_parts creates slice [0..len)
        // - Slice lifetime bound to &self
        unsafe { std::slice::from_raw_parts(self.data, self.len) }
    }

    /// Get a mutable slice of the vector
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: Creating mutable slice from vector data.
        // - data is valid pointer (allocated from arena)
        // - len represents number of initialized elements
        // - from_raw_parts_mut creates slice [0..len)
        // - &mut self ensures exclusive access
        // - Slice lifetime bound to &mut self
        unsafe { std::slice::from_raw_parts_mut(self.data, self.len) }
    }
}

impl<T, A: ArenaAllocate> Drop for ArenaBackedVec<T, A> {
    fn drop(&mut self) {
        self.clear();
        // Memory is managed by the arena, so we don't deallocate here
    }
}

impl<T, A: ArenaAllocate> std::ops::Index<usize> for ArenaBackedVec<T, A> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("index out of bounds")
    }
}

impl<T, A: ArenaAllocate> std::ops::IndexMut<usize> for ArenaBackedVec<T, A> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("index out of bounds")
    }
}

impl<T: std::fmt::Debug, A: ArenaAllocate> std::fmt::Debug for ArenaBackedVec<T, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T: PartialEq, A: ArenaAllocate> PartialEq for ArenaBackedVec<T, A> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq, A: ArenaAllocate> Eq for ArenaBackedVec<T, A> {}

/// A string that uses arena allocation
pub type ArenaString<A> = ArenaBackedVec<u8, A>;

impl<A: ArenaAllocate> ArenaString<A> {
    /// Create a new arena string from a str
    #[must_use = "allocated string must be used"]
    pub fn from_str(s: &str, allocator: Arc<A>) -> Result<Self, MemoryError> {
        let mut vec = Self::with_capacity(s.len(), allocator)?;
        for &byte in s.as_bytes() {
            vec.push(byte)?;
        }
        Ok(vec)
    }

    /// Get the string as a str
    #[must_use]
    pub fn as_str(&self) -> &str {
        // SAFETY: Creating &str from bytes.
        // - ArenaString constructed from valid UTF-8 in from_str
        // - as_slice() returns valid byte slice
        // - UTF-8 validity preserved (no mutation outside from_str)
        // - from_utf8_unchecked safe with guaranteed UTF-8 bytes
        unsafe { std::str::from_utf8_unchecked(self.as_slice()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_allocator_basic() {
        let allocator = ArenaAllocator::new_basic();

        let value = allocator.alloc(42u32).unwrap();
        assert_eq!(*value, 42);

        let slice = allocator.alloc_slice_copy(&[1, 2, 3, 4, 5]).unwrap();
        assert_eq!(slice, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_arena_backed_vec() {
        let arena = Arc::new(Arena::new(Default::default()));
        let mut vec = ArenaBackedVec::with_capacity(10, arena).unwrap();

        vec.push(1).unwrap();
        vec.push(2).unwrap();
        vec.push(3).unwrap();

        assert_eq!(vec.len(), 3);
        assert_eq!(vec.capacity(), 10);
        assert_eq!(vec[0], 1);
        assert_eq!(vec[1], 2);
        assert_eq!(vec[2], 3);

        assert_eq!(vec.pop(), Some(3));
        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn test_arena_backed_vec_clear() {
        let arena = Arc::new(Arena::new(Default::default()));
        let mut vec = ArenaBackedVec::with_capacity(5, arena).unwrap();

        for i in 0..5 {
            vec.push(i).unwrap();
        }

        assert_eq!(vec.len(), 5);
        vec.clear();
        assert_eq!(vec.len(), 0);
        assert!(vec.is_empty());
    }

    #[test]
    fn test_arena_string() {
        let arena = Arc::new(Arena::new(Default::default()));
        let string = ArenaString::from_str("Hello, Arena!", arena).unwrap();

        assert_eq!(string.as_str(), "Hello, Arena!");
        assert_eq!(string.len(), 13);
    }

    #[test]
    fn test_thread_safe_allocator() {
        use std::thread;

        let allocator = ArenaAllocator::new_thread_safe();

        let handles: Vec<_> = (0..4)
            .map(|i| {
                let allocator = allocator.clone();
                thread::spawn(move || {
                    let value = allocator.alloc(i * 100).unwrap();
                    *value
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        assert_eq!(results, vec![0, 100, 200, 300]);
    }

    #[test]
    fn test_allocator_with_layout() {
        let allocator = ArenaAllocator::new_basic();

        let layout = Layout::from_size_align(64, 16).unwrap();
        let ptr = unsafe { allocator.allocate(layout).unwrap() };

        // NonNull<u8> guarantees ptr is not null by type invariant
        assert_eq!(ptr.as_ptr() as usize % 16, 0); // Check alignment
    }
}
