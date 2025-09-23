//! High-performance, thread-unsafe bump allocator arena

use std::alloc::{alloc, dealloc, Layout};
use std::cell::{Cell, RefCell};
use std::mem::{self, MaybeUninit};
use std::ptr::{self, NonNull};
use std::time::Instant;

use super::{ArenaAllocate, ArenaConfig, ArenaStats};
use crate::error::MemoryError;
use crate::utils::align_up;

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
            .map_err(|_| MemoryError::InvalidLayout { reason: "size overflow" })?;

        // Safety: Layout is non-zero size and properly aligned
        let ptr = unsafe { alloc(layout) };
        let ptr =
            NonNull::new(ptr).ok_or(MemoryError::OutOfMemory { requested: size, available: 0 })?;

        Ok(Self { ptr, capacity: size, next: None })
    }

    #[inline]
    fn start(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    #[inline]
    fn end(&self) -> *mut u8 {
        unsafe { self.ptr.as_ptr().add(self.capacity) }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), Layout::from_size_align_unchecked(self.capacity, 1));
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

    /// Allocates new chunk of memory
    fn allocate_chunk(&self, min_size: usize) -> Result<(), MemoryError> {
        let mut chunks = self.chunks.borrow_mut();

        let chunk_size = match &*chunks {
            Some(chunk) => {
                // Calculate next chunk size using growth factor
                let new_size = (chunk.capacity as f64 * self.config.growth_factor) as usize;
                new_size.max(min_size).min(self.config.max_chunk_size)
            },
            None => self.config.initial_size.max(min_size),
        };

        let mut new_chunk = Chunk::new(chunk_size)?;

        // Zero memory if requested
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
    pub fn alloc_bytes_aligned(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if !align.is_power_of_two() {
            return Err(MemoryError::InvalidAlignment { required: align, actual: 0 });
        }

        let start_time = self.config.track_stats.then(Instant::now);

        // Calculate aligned pointer and padding needed
        let current = self.current_ptr.get();
        let aligned = align_up(current as usize, align) as *mut u8;
        let padding = aligned as usize - current as usize;

        // Check if we need a new chunk
        let needed = size + padding;
        if current.is_null() || unsafe { aligned.add(size) > self.current_end.get() } {
            self.allocate_chunk(needed)?;
            return self.alloc_bytes_aligned(size, align);
        }

        // Update bump pointer
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
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        let ptr = self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())? as *mut T;

        // Safety: We just allocated properly aligned space for T
        unsafe {
            ptr.write(value);
            Ok(&mut *ptr)
        }
    }

    /// Allocates space for uninitialized value
    pub fn alloc_uninit<T>(&self) -> Result<&mut MaybeUninit<T>, MemoryError> {
        let ptr = self.alloc_bytes_aligned(mem::size_of::<T>(), mem::align_of::<T>())?
            as *mut MaybeUninit<T>;

        // Safety: We just allocated properly aligned space for MaybeUninit<T>
        unsafe { Ok(&mut *ptr) }
    }

    /// Allocates and copies a slice
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<&mut [T], MemoryError> {
        if slice.is_empty() {
            return Ok(&mut []);
        }

        let ptr =
            self.alloc_bytes_aligned(mem::size_of_val(slice), mem::align_of::<T>())? as *mut T;

        // Safety: We just allocated properly aligned space for the slice
        unsafe {
            ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(&mut *ptr::slice_from_raw_parts_mut(ptr, slice.len()))
        }
    }

    /// Allocates a string
    pub fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        let bytes = self.alloc_slice(s.as_bytes())?;
        // Safety: We know the bytes are valid UTF-8 since they came from &str
        unsafe { Ok(std::str::from_utf8_unchecked(bytes)) }
    }

    /// Resets the arena while retaining allocated chunks
    pub fn reset(&mut self) {
        let start_time = self.config.track_stats.then(Instant::now);

        if let Some(chunk) = &*self.chunks.borrow() {
            self.current_ptr.set(chunk.start());
            self.current_end.set(chunk.end());

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
        unsafe { self.ptr.as_ref() }
    }

    /// Gets mutable reference to the value
    pub fn get_mut(&mut self) -> &mut T {
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
        let config = ArenaConfig::default().with_initial_size(128).with_growth_factor(2.0);

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
