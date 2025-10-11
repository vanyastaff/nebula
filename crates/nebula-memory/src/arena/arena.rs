//! High-performance, thread-unsafe bump allocator arena
//!
//! # Safety
//!
//! This module implements a single-threaded bump allocator arena:
//! - RefCell for chunk list mutation (runtime borrow checking)
//! - Cell for current pointer (no synchronization, single-threaded)
//! - Linked list of chunks (grows on demand via growth_factor)
//! - Position markers enable scoped resets
//!
//! ## Invariants
//!
//! - current_ptr always within [chunk.start(), chunk.end()] or null
//! - Allocations never overlap (bump pointer moves forward monotonically)
//! - Chunks linked in reverse allocation order (newest first)
//! - Position validation ensures chunk_ptr matches current arena state
//! - reset_to_position verifies offset doesn't exceed current_ptr
//!
//! ## Memory Management
//!
//! - Chunks allocated via std::alloc::alloc with Layout
//! - Chunks deallocated in Drop via std::alloc::dealloc
//! - Growth factor determines next chunk size (exponential growth)
//! - Optional zero_memory for security/debugging
//! - No individual deallocation (arena discipline)
//!
//! ## Not Thread-Safe
//!
//! - Uses Cell/RefCell instead of atomics
//! - No Send/Sync implementations
//! - Designed for single-threaded high performance
//! - See ThreadSafeArena for multi-threaded version

use std::alloc::{Layout, alloc, dealloc};
use std::cell::{Cell, RefCell};
use std::mem::{self, MaybeUninit};
use std::ptr::{self, NonNull};
use std::time::Instant;

use super::{ArenaAllocate, ArenaConfig, ArenaStats};
use crate::error::MemoryError;
use crate::utils::align_up;

/// Position marker for arena state
///
/// This opaque type represents a specific position in an arena's allocation
/// history. It can be used with `reset_to_position` to restore the arena
/// to a previous state.
///
/// # Safety
///
/// Positions must only be used with the arena they were created from.
/// Using a position from a different arena will result in undefined behavior.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Position {
    /// Current pointer offset from chunk start
    offset: usize,
    /// Pointer to verify arena identity
    chunk_ptr: *const u8,
}

impl Position {
    /// Creates a new position marker
    #[inline]
    fn new(offset: usize, chunk_ptr: *const u8) -> Self {
        Self { offset, chunk_ptr }
    }
}

// SAFETY: Position is just a marker containing primitive values.
// - offset is usize (Copy, no ownership)
// - chunk_ptr is *const u8 (raw pointer, no ownership)
// - Position doesn't manage memory, just stores verification data
// - Safe to send between threads (though Arena itself is !Send)
unsafe impl Send for Position {}

// SAFETY: Position can be shared between threads.
// - All fields are Copy primitives
// - No interior mutability
// - Read-only verification token
unsafe impl Sync for Position {}

/// Memory chunk managed by the arena
struct Chunk {
    ptr: NonNull<u8>,
    capacity: usize,
    next: Option<Box<Chunk>>,
}

impl Chunk {
    /// Creates a new chunk with specified size
    fn new(size: usize) -> Result<Self, MemoryError> {
        // Ensure minimum chunk size to reduce fragmentation
        let size = size.max(64); // Minimum 64 bytes

        let layout = Layout::from_size_align(size, 1)
            .map_err(|_| MemoryError::invalid_layout("layout creation failed"))?;

        // SAFETY: Allocating memory via global allocator.
        // - layout is valid (non-zero size, align=1 is always valid)
        // - size >= 64 (ensured above)
        // - alloc returns null on failure (handled below)
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).ok_or_else(|| MemoryError::out_of_memory(size, 0))?;

        Ok(Self {
            ptr,
            capacity: size,
            next: None,
        })
    }

    #[inline]
    fn start(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    #[inline]
    fn end(&self) -> *mut u8 {
        // SAFETY: Computing end pointer via offset.
        // - ptr is valid (allocated in new())
        // - capacity is the allocation size
        // - add(capacity) gives one-past-end pointer (valid for comparison)
        unsafe { self.ptr.as_ptr().add(self.capacity) }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: Deallocating chunk memory.
        // - ptr was allocated via alloc() in new() with same layout
        // - Layout::from_size_align_unchecked is safe (capacity and align=1 are valid)
        // - capacity matches original allocation
        // - This is called exactly once (Drop guarantee)
        unsafe {
            dealloc(
                self.ptr.as_ptr(),
                Layout::from_size_align_unchecked(self.capacity, 1),
            );
        }
    }
}

/// High-performance bump allocator arena
pub struct Arena {
    chunks: RefCell<Option<Box<Chunk>>>,
    current_ptr: Cell<*mut u8>,
    current_end: Cell<*mut u8>,
    config: ArenaConfig,
    stats: ArenaStats,
}

impl Arena {
    /// Creates new arena with specified configuration
    pub fn new(config: ArenaConfig) -> Self {
        Self {
            chunks: RefCell::new(None),
            current_ptr: Cell::new(ptr::null_mut()),
            current_end: Cell::new(ptr::null_mut()),
            config,
            stats: ArenaStats::new(),
        }
    }

    /// Creates new arena with default config and minimum capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(ArenaConfig::default().with_initial_size(capacity))
    }

    /// Creates arena with production config - optimized for performance
    pub fn production(capacity: usize) -> Self {
        Self::new(ArenaConfig::production().with_initial_size(capacity))
    }

    /// Creates arena with debug config - optimized for debugging
    pub fn debug(capacity: usize) -> Self {
        Self::new(ArenaConfig::debug().with_initial_size(capacity))
    }

    /// Creates arena with performance config (alias for production)
    pub fn performance(capacity: usize) -> Self {
        Self::production(capacity)
    }

    /// Creates arena with conservative config - balanced
    pub fn conservative(capacity: usize) -> Self {
        Self::new(ArenaConfig::conservative().with_initial_size(capacity))
    }

    /// Creates arena optimized for small frequent allocations
    pub fn small_objects(capacity: usize) -> Self {
        Self::new(ArenaConfig::small_objects().with_initial_size(capacity))
    }

    /// Creates arena optimized for large infrequent allocations
    pub fn large_objects(capacity: usize) -> Self {
        Self::new(ArenaConfig::large_objects().with_initial_size(capacity))
    }

    /// Creates a tiny arena (4KB) for testing or minimal use
    pub fn tiny() -> Self {
        Self::new(ArenaConfig::small_objects().with_initial_size(4 * 1024))
    }

    /// Creates a small arena (64KB) for common use
    pub fn small() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(64 * 1024))
    }

    /// Creates a medium arena (1MB) for standard applications
    pub fn medium() -> Self {
        Self::new(ArenaConfig::default().with_initial_size(1024 * 1024))
    }

    /// Creates a large arena (16MB) for heavy workloads
    pub fn large() -> Self {
        Self::new(ArenaConfig::large_objects().with_initial_size(16 * 1024 * 1024))
    }

    /// Allocates new chunk of memory
    fn allocate_chunk(&self, min_size: usize) -> Result<(), MemoryError> {
        let mut chunks = self.chunks.borrow_mut();

        let chunk_size = match &*chunks {
            Some(chunk) => {
                // Calculate next chunk size using growth factor
                let new_size = (chunk.capacity as f64 * self.config.growth_factor) as usize;
                new_size.max(min_size).min(self.config.max_chunk_size)
            }
            None => self.config.initial_size.max(min_size),
        };

        let mut new_chunk = Chunk::new(chunk_size)?;

        // Zero memory if requested
        // SAFETY: Zeroing newly allocated chunk.
        // - new_chunk.start() points to valid allocated memory
        // - chunk_size is the allocation size
        // - Memory is ours to initialize (just allocated)
        if self.config.zero_memory {
            unsafe {
                ptr::write_bytes(new_chunk.start(), 0, chunk_size);
            }
        }

        // Update allocation pointers
        self.current_ptr.set(new_chunk.start());
        self.current_end.set(new_chunk.end());

        // Prepend new chunk to list
        new_chunk.next = chunks.take();
        *chunks = Some(Box::new(new_chunk));

        // Update statistics if enabled
        if self.config.track_stats {
            self.stats.record_chunk_allocation(chunk_size);
        }

        Ok(())
    }

    /// Allocates aligned memory block
    #[must_use = "allocated memory must be used"]
    pub fn alloc_bytes_aligned(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if !align.is_power_of_two() {
            return Err(MemoryError::invalid_alignment(align));
        }

        let start_time = self.config.track_stats.then(Instant::now);

        // Calculate aligned pointer and padding needed
        let current = self.current_ptr.get();
        let aligned = align_up(current as usize, align) as *mut u8;
        let padding = aligned as usize - current as usize;

        // Check if we need a new chunk
        let needed = size + padding;
        // SAFETY: Computing end of allocation.
        // - aligned is valid address (computed from current or will be set by allocate_chunk)
        // - add(size) computes one-past-end of allocation
        // - Comparing with current_end to check bounds
        if current.is_null() || unsafe { aligned.add(size) > self.current_end.get() } {
            self.allocate_chunk(needed)?;
            return self.alloc_bytes_aligned(size, align);
        }

        // Update bump pointer
        // SAFETY: Advancing bump pointer after allocation.
        // - aligned is within current chunk (checked above)
        // - add(size) is within bounds (checked above: aligned + size <= current_end)
        // - This becomes the new current_ptr for next allocation
        self.current_ptr.set(unsafe { aligned.add(size) });

        // Update statistics if enabled
        if let Some(start) = start_time {
            let elapsed = start.elapsed().as_nanos() as u64;
            self.stats.record_allocation(size, elapsed);
            if padding > 0 {
                self.stats.record_waste(padding);
            }
        }

        Ok(aligned)
    }

    /// Allocates and initializes a value
    #[must_use = "allocated memory must be used"]
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        let ptr = self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())? as *mut T;

        // SAFETY: Initializing allocated memory and creating reference.
        // - ptr is valid (just allocated via alloc_bytes_aligned)
        // - ptr is properly aligned for T (alloc_bytes_aligned guarantees)
        // - ptr has space for T (size_of::<T>() bytes allocated)
        // - write moves value into allocated memory
        // - After write, memory contains valid T
        // - Reference lifetime bound to &self (arena lifetime)
        unsafe {
            ptr.write(value);
            Ok(&mut *ptr)
        }
    }

    /// Allocates space for uninitialized value
    #[must_use = "allocated memory must be used"]
    pub fn alloc_uninit<T>(&self) -> Result<&mut MaybeUninit<T>, MemoryError> {
        let ptr = self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())?
            as *mut MaybeUninit<T>;

        // SAFETY: Creating reference to uninitialized memory.
        // - ptr is valid (just allocated via alloc_bytes_aligned)
        // - ptr is properly aligned for MaybeUninit<T> (same layout as T)
        // - ptr has space for MaybeUninit<T> (size_of::<T>() bytes)
        // - MaybeUninit<T> allows uninitialized memory
        // - Reference lifetime bound to &self (arena lifetime)
        unsafe { Ok(&mut *ptr) }
    }

    /// Allocates and copies a slice
    #[must_use = "allocated memory must be used"]
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<&mut [T], MemoryError> {
        if slice.is_empty() {
            return Ok(&mut []);
        }

        let ptr =
            self.alloc_bytes_aligned(mem::size_of_val(slice), mem::align_of::<T>())? as *mut T;

        // SAFETY: Copying slice to allocated memory and creating slice reference.
        // - ptr is valid (just allocated via alloc_bytes_aligned)
        // - ptr is properly aligned for T
        // - ptr has space for slice.len() elements (size_of_val bytes allocated)
        // - copy_nonoverlapping copies slice.len() elements
        // - Source (slice) and dest (ptr) don't overlap (new allocation)
        // - After copy, memory contains valid T instances (T: Copy)
        // - slice_from_raw_parts_mut creates slice reference
        // - Reference lifetime bound to &self (arena lifetime)
        unsafe {
            ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(&mut *ptr::slice_from_raw_parts_mut(ptr, slice.len()))
        }
    }

    /// Allocates a string
    #[must_use = "allocated memory must be used"]
    pub fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        let bytes = self.alloc_slice(s.as_bytes())?;
        // SAFETY: Creating &str from bytes.
        // - bytes contains valid UTF-8 (copied from &str in alloc_slice)
        // - alloc_slice preserves byte values exactly (Copy trait)
        // - UTF-8 validity is preserved through copy
        // - Reference lifetime bound to &self (arena lifetime)
        unsafe { Ok(std::str::from_utf8_unchecked(bytes)) }
    }

    /// Resets the arena while retaining allocated chunks
    pub fn reset(&mut self) {
        let start_time = self.config.track_stats.then(Instant::now);

        if let Some(chunk) = &*self.chunks.borrow() {
            self.current_ptr.set(chunk.start());
            self.current_end.set(chunk.end());

            // SAFETY: Zeroing chunk on reset.
            // - chunk.start() is valid (points to allocated chunk)
            // - chunk.capacity is the allocation size
            // - Caller has &mut self, ensuring no outstanding references
            // - Safe to overwrite all memory in chunk
            if self.config.zero_memory {
                unsafe {
                    ptr::write_bytes(chunk.start(), 0, chunk.capacity);
                }
            }
        } else {
            self.current_ptr.set(ptr::null_mut());
            self.current_end.set(ptr::null_mut());
        }

        if let Some(start) = start_time {
            self.stats.record_reset(start.elapsed().as_nanos() as u64);
        }
    }

    /// Returns the current position in the arena
    ///
    /// This position can be used with `reset_to_position` to restore the arena
    /// to its current state, effectively creating a scoped allocation region.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_memory::arena::{Arena, ArenaConfig};
    ///
    /// let arena = Arena::new(ArenaConfig::default());
    /// let pos = arena.current_position();
    /// let _value = arena.alloc(42).unwrap();
    /// // Can reset back to pos later
    /// ```
    #[must_use]
    pub fn current_position(&self) -> Position {
        let chunks = self.chunks.borrow();
        let chunk_ptr = chunks
            .as_ref()
            .map(|c| c.start() as *const u8)
            .unwrap_or(ptr::null());

        let offset = if chunk_ptr.is_null() {
            0
        } else {
            let current = self.current_ptr.get();
            if current.is_null() {
                0
            } else {
                (current as usize).saturating_sub(chunk_ptr as usize)
            }
        };

        Position::new(offset, chunk_ptr)
    }

    /// Resets the arena to a previously saved position
    ///
    /// This allows for scoped allocations where temporary allocations can be
    /// rolled back to a specific point.
    ///
    /// # Safety
    ///
    /// The position must have been created from this arena instance. Using a
    /// position from a different arena will result in undefined behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The position is from a different arena
    /// - The position is invalid (offset beyond current position)
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_memory::arena::{Arena, ArenaConfig};
    ///
    /// let mut arena = Arena::new(ArenaConfig::default());
    /// let pos = arena.current_position();
    /// let _temp = arena.alloc(42).unwrap();
    /// arena.reset_to_position(pos).unwrap();
    /// // Temporary allocation is now invalid
    /// ```
    pub fn reset_to_position(&mut self, position: Position) -> Result<(), MemoryError> {
        let chunks = self.chunks.borrow();

        // Verify position is from this arena
        let chunk_ptr = chunks
            .as_ref()
            .map(|c| c.start() as *const u8)
            .unwrap_or(ptr::null());

        if position.chunk_ptr != chunk_ptr {
            return Err(MemoryError::invalid_argument(
                "Position is from a different arena or arena has been reset",
            ));
        }

        // Calculate new pointer
        let new_ptr = if chunk_ptr.is_null() {
            ptr::null_mut()
        } else {
            // SAFETY: Computing position pointer via offset.
            // - chunk_ptr is valid (verified to match current chunk above)
            // - position.offset will be validated below (not beyond current_ptr)
            // - Pointer arithmetic within chunk bounds (validated in next block)
            unsafe { (chunk_ptr as *mut u8).add(position.offset) }
        };

        // Validate offset is within bounds
        if !new_ptr.is_null() {
            let current = self.current_ptr.get();
            if !current.is_null() && new_ptr > current {
                return Err(MemoryError::invalid_argument(
                    "Position offset is beyond current allocation point",
                ));
            }
        }

        // Zero memory if requested
        // SAFETY: Zeroing memory being freed by reset_to_position.
        // - new_ptr is valid (validated above to be within chunk)
        // - current is valid (current allocation pointer)
        // - new_ptr < current means freeing range [new_ptr, current)
        // - size is the range length
        // - Caller has &mut self, ensuring no outstanding references to this range
        if self.config.zero_memory && !new_ptr.is_null() {
            let current = self.current_ptr.get();
            if !current.is_null() && new_ptr < current {
                let size = current as usize - new_ptr as usize;
                unsafe {
                    ptr::write_bytes(new_ptr, 0, size);
                }
            }
        }

        // Update pointer
        self.current_ptr.set(new_ptr);

        Ok(())
    }

    /// Returns reference to statistics
    pub fn stats(&self) -> &ArenaStats {
        &self.stats
    }
}

/// Reference to an arena-allocated value
pub struct ArenaRef<'a, T: ?Sized> {
    ptr: NonNull<T>,
    _arena: &'a Arena,
}

impl<'a, T: ?Sized> ArenaRef<'a, T> {
    /// Creates a new reference from raw pointer
    pub(crate) fn new(ptr: NonNull<T>, arena: &'a Arena) -> Self {
        Self { ptr, _arena: arena }
    }

    /// Gets reference to the value
    pub fn get(&self) -> &T {
        // SAFETY: Dereferencing arena-allocated pointer.
        // - ptr is NonNull, guaranteed non-null
        // - ptr points to valid T in arena (created by ArenaRef::new)
        // - _arena field ensures reference doesn't outlive arena
        // - Reference lifetime bound to &self
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T: ?Sized> std::ops::Deref for ArenaRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

/// Mutable reference to an arena-allocated value
pub struct ArenaRefMut<'a, T: ?Sized> {
    ptr: NonNull<T>,
    _arena: &'a Arena,
}

impl<'a, T: ?Sized> ArenaRefMut<'a, T> {
    /// Creates a new mutable reference from raw pointer
    pub(crate) fn new(ptr: NonNull<T>, arena: &'a Arena) -> Self {
        Self { ptr, _arena: arena }
    }

    /// Gets reference to the value
    pub fn get(&self) -> &T {
        // SAFETY: Dereferencing arena-allocated pointer (immutable).
        // - ptr is NonNull, guaranteed non-null
        // - ptr points to valid T in arena (created by ArenaRefMut::new)
        // - _arena field ensures reference doesn't outlive arena
        // - Reference lifetime bound to &self
        unsafe { self.ptr.as_ref() }
    }

    /// Gets mutable reference to the value
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: Dereferencing arena-allocated pointer (mutable).
        // - ptr is NonNull, guaranteed non-null
        // - ptr points to valid T in arena (created by ArenaRefMut::new)
        // - &mut self ensures exclusive access (no aliasing)
        // - _arena field ensures reference doesn't outlive arena
        // - Reference lifetime bound to &mut self
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T: ?Sized> std::ops::Deref for ArenaRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<'a, T: ?Sized> std::ops::DerefMut for ArenaRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl ArenaAllocate for Arena {
    unsafe fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        // SAFETY: Forwarding to alloc_bytes_aligned.
        // - Same safety contract as ArenaAllocate::alloc_bytes
        // - Caller guarantees size and align are valid (trait contract)
        // - alloc_bytes_aligned handles all safety concerns
        self.alloc_bytes_aligned(size, align)
    }

    fn stats(&self) -> &ArenaStats {
        &self.stats
    }

    fn reset(&mut self) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::is_aligned;

    #[test]
    fn basic_allocation() {
        let arena = Arena::new(ArenaConfig::default());
        let value = arena.alloc(42u32).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn alignment_requirements() {
        let arena = Arena::new(ArenaConfig::default());

        let p1 = arena.alloc_bytes_aligned(1, 1).unwrap();
        assert!(is_aligned(p1 as usize, 1));

        let p64 = arena.alloc_bytes_aligned(1, 64).unwrap();
        assert!(is_aligned(p64 as usize, 64));
    }

    #[test]
    fn chunk_growth() {
        let config = ArenaConfig::default()
            .with_initial_size(128)
            .with_growth_factor(2.0);

        let arena = Arena::new(config);

        // First allocation fits in initial chunk
        let _ = arena.alloc_bytes_aligned(64, 1).unwrap();

        // This should trigger chunk growth
        let _ = arena.alloc_bytes_aligned(256, 1).unwrap();

        assert!(arena.stats().chunks_allocated() > 1);
    }

    #[test]
    fn reset_behavior() {
        let mut arena = Arena::new(ArenaConfig::default().with_stats(true));

        let _ = arena.alloc(1u32).unwrap();
        let _ = arena.alloc(2u32).unwrap();

        assert_eq!(arena.stats().allocations(), 2);

        arena.reset();

        assert_eq!(arena.stats().allocations(), 0);
        assert_eq!(arena.stats().resets(), 1);
    }

    #[test]
    fn edge_cases() {
        let arena = Arena::new(ArenaConfig::default().with_initial_size(8));

        // Test allocation of zero bytes
        let ptr = arena.alloc_bytes_aligned(0, 1).unwrap();
        assert!(!ptr.is_null());

        // Test allocation with large alignment
        let ptr = arena.alloc_bytes_aligned(1, 4096).unwrap();
        assert!(is_aligned(ptr as usize, 4096));
    }
}
