//! Arena-based allocator implementation
//!
//! This module provides the [`ArenaAllocator`] type, which can be used to
//! allocate memory from an arena. It also provides the [`ArenaBackedVec`] type
//! for arena-allocated vectors.

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
        unsafe {
            let ptr = self.arena.alloc_bytes(layout.size(), layout.align())?;
            NonNull::new(ptr).ok_or_else(|| MemoryError::allocation_failed())
        }
    }

    /// Allocate a slice of memory with the given layout
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is properly initialized before use
    pub unsafe fn allocate_slice(&self, layout: Layout) -> Result<NonNull<[u8]>, MemoryError> {
        unsafe {
            let ptr = self.arena.alloc_bytes(layout.size(), layout.align())?;

            // Create a slice from the allocated memory
            let slice = std::slice::from_raw_parts_mut(ptr, layout.size());

            Ok(NonNull::from(slice))
        }
    }

    /// Allocate and initialize a value
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        self.arena.alloc(value)
    }

    /// Allocate and initialize a slice
    pub fn alloc_slice_copy<T: Copy>(&self, slice: &[T]) -> Result<&mut [T], MemoryError> {
        self.arena.alloc_slice(slice)
    }

    /// Create a new vector backed by this arena
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
    pub fn new_basic() -> Self {
        Self::new(Arc::new(Arena::new(Default::default())))
    }

    /// Create a new allocator with a basic arena with specified capacity
    pub fn new_basic_with_capacity(capacity: usize) -> Self {
        Self::new(Arc::new(Arena::with_capacity(capacity)))
    }
}

impl ArenaAllocator<ThreadSafeArena> {
    /// Create a new allocator with a thread-safe arena
    pub fn new_thread_safe() -> Self {
        Self::new(Arc::new(ThreadSafeArena::new(Default::default())))
    }

    /// Create a new allocator with a thread-safe arena with specified config
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
    pub fn with_capacity(capacity: usize, allocator: Arc<A>) -> Result<Self, MemoryError> {
        if capacity == 0 {
            return Ok(Self::new(allocator));
        }

        let layout = Layout::array::<T>(capacity).map_err(|_| MemoryError::invalid_layout())?;

        let ptr = unsafe { allocator.alloc_bytes(layout.size(), layout.align())? };

        Ok(Self {
            data: ptr as *mut T,
            len: 0,
            capacity,
            allocator,
            _marker: PhantomData,
        })
    }

    /// Push a value onto the vector
    pub fn push(&mut self, value: T) -> Result<(), MemoryError> {
        if self.len >= self.capacity {
            return Err(MemoryError::allocation_too_large(self.len + 1));
        }

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
        unsafe { Some(self.data.add(self.len).read()) }
    }

    /// Get a reference to an element
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len {
            unsafe { Some(&*self.data.add(index)) }
        } else {
            None
        }
    }

    /// Get a mutable reference to an element
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            unsafe { Some(&mut *self.data.add(index)) }
        } else {
            None
        }
    }

    /// Returns the length of the vector
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the vector is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the capacity of the vector
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear the vector
    pub fn clear(&mut self) {
        unsafe {
            // Drop all elements
            for i in 0..self.len {
                std::ptr::drop_in_place(self.data.add(i));
            }
        }
        self.len = 0;
    }

    /// Get a slice of the vector
    pub fn as_slice(&self) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.data, self.len) }
    }

    /// Get a mutable slice of the vector
    pub fn as_mut_slice(&mut self) -> &mut [T] {
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
    pub fn from_str(s: &str, allocator: Arc<A>) -> Result<Self, MemoryError> {
        let mut vec = Self::with_capacity(s.len(), allocator)?;
        for &byte in s.as_bytes() {
            vec.push(byte)?;
        }
        Ok(vec)
    }

    /// Get the string as a str
    pub fn as_str(&self) -> &str {
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

        assert!(!ptr.as_ptr().is_null());
        assert_eq!(ptr.as_ptr() as usize % 16, 0); // Check alignment
    }
}
