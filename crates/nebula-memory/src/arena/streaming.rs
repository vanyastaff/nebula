//! Streaming arena implementation for sequential data processing

use std::alloc::{alloc, dealloc, Layout};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::ptr::NonNull;

use super::ArenaStats;
use crate::core::error::MemoryError;
use crate::utils::align_up;

/// Configuration for streaming arena
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Size of each buffer in bytes
    pub buffer_size: usize,

    /// Maximum number of buffers to keep
    pub max_buffers: usize,

    /// Whether to recycle old buffers
    pub recycle_buffers: bool,

    /// Alignment for allocations
    pub alignment: usize,

    /// Whether to track statistics
    pub track_stats: bool,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            buffer_size: 64 * 1024, // 64KB
            max_buffers: 16,
            recycle_buffers: true,
            alignment: 8,
            track_stats: cfg!(debug_assertions),
        }
    }
}

/// A buffer in the streaming arena
struct StreamBuffer {
    ptr: NonNull<u8>,
    capacity: usize,
    used: Cell<usize>,
}

impl StreamBuffer {
    fn new(size: usize) -> Result<Self, MemoryError> {
        let layout = Layout::from_size_align(size, 1).map_err(|_| MemoryError::invalid_layout())?;

        let ptr = unsafe { alloc(layout) };

        match NonNull::new(ptr) {
            Some(ptr) => Ok(StreamBuffer { ptr, capacity: size, used: Cell::new(0) }),
            None => Err(MemoryError::allocation_failed()),
        }
    }

    fn reset(&self) {
        self.used.set(0);
    }

    fn available(&self) -> usize {
        self.capacity - self.used.get()
    }

    fn try_alloc(&self, size: usize, align: usize) -> Option<*mut u8> {
        let current = self.used.get();
        let aligned = align_up(current, align);
        let needed = aligned - current + size;

        if needed <= self.available() {
            self.used.set(aligned + size);
            Some(unsafe { self.ptr.as_ptr().add(aligned) })
        } else {
            None
        }
    }
}

impl Drop for StreamBuffer {
    fn drop(&mut self) {
        unsafe {
            let layout = Layout::from_size_align_unchecked(self.capacity, 1);
            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

/// Streaming arena optimized for sequential allocation patterns
///
/// This arena is designed for workloads that process data in streams,
/// allocating memory for each item and then moving to the next.
pub struct StreamingArena<T = ()> {
    active_buffers: RefCell<VecDeque<StreamBuffer>>,
    free_buffers: RefCell<Vec<StreamBuffer>>,
    current_buffer: RefCell<Option<usize>>,
    options: StreamOptions,
    stats: ArenaStats,
    _phantom: PhantomData<T>,
}

impl<T> StreamingArena<T> {
    /// Create a new streaming arena
    pub fn new(options: StreamOptions) -> Self {
        StreamingArena {
            active_buffers: RefCell::new(VecDeque::new()),
            free_buffers: RefCell::new(Vec::new()),
            current_buffer: RefCell::new(None),
            options,
            stats: ArenaStats::new(),
            _phantom: PhantomData,
        }
    }

    /// Create with default options
    pub fn default() -> Self {
        Self::new(StreamOptions::default())
    }

    /// Get or create a new buffer
    fn get_buffer(&self) -> Result<(), MemoryError> {
        // Try to reuse a free buffer
        if self.options.recycle_buffers {
            if let Some(buffer) = self.free_buffers.borrow_mut().pop() {
                buffer.reset();
                self.active_buffers.borrow_mut().push_back(buffer);
                let index = self.active_buffers.borrow().len() - 1;
                *self.current_buffer.borrow_mut() = Some(index);
                return Ok(());
            }
        }

        // Check buffer limit
        if self.active_buffers.borrow().len() >= self.options.max_buffers {
            // Recycle oldest buffer if recycling is enabled
            if self.options.recycle_buffers {
                if let Some(mut buffer) = self.active_buffers.borrow_mut().pop_front() {
                    buffer.reset();
                    self.active_buffers.borrow_mut().push_back(buffer);
                    let index = self.active_buffers.borrow().len() - 1;
                    *self.current_buffer.borrow_mut() = Some(index);
                    return Ok(());
                }
            }
            return Err(MemoryError::out_of_memory(self.options.buffer_size, 0));
        }

        // Allocate new buffer
        let buffer = StreamBuffer::new(self.options.buffer_size)?;

        // Update stats
        if self.options.track_stats {
            self.stats.record_chunk_allocation(self.options.buffer_size);
        }

        self.active_buffers.borrow_mut().push_back(buffer);
        let index = self.active_buffers.borrow().len() - 1;
        *self.current_buffer.borrow_mut() = Some(index);

        Ok(())
    }

    /// Allocate bytes in the streaming arena
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if size > self.options.buffer_size {
            return Err(MemoryError::allocation_too_large(0));
        }

        let start_time =
            if self.options.track_stats { Some(std::time::Instant::now()) } else { None };

        // Try current buffer
        if let Some(index) = *self.current_buffer.borrow() {
            let buffers = self.active_buffers.borrow();
            if let Some(buffer) = buffers.get(index) {
                if let Some(ptr) = buffer.try_alloc(size, align) {
                    // Update stats
                    if self.options.track_stats {
                        let elapsed = start_time.unwrap().elapsed().as_nanos() as u64;
                        self.stats.record_allocation(size, elapsed);
                    }
                    return Ok(ptr);
                }
            }
        }

        // Need new buffer
        self.get_buffer()?;

        // Retry allocation
        let buffers = self.active_buffers.borrow();
        let index = self.current_buffer.borrow().unwrap();
        let buffer = &buffers[index];

        match buffer.try_alloc(size, align) {
            Some(ptr) => {
                // Update stats
                if self.options.track_stats {
                    let elapsed = start_time.unwrap().elapsed().as_nanos() as u64;
                    self.stats.record_allocation(size, elapsed);
                }
                Ok(ptr)
            },
            None => Err(MemoryError::allocation_failed()),
        }
    }

    /// Allocate a value
    pub fn alloc(&self, value: T) -> Result<StreamingArenaRef<T>, MemoryError> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();

        let ptr = self.alloc_bytes(size, align)? as *mut T;

        unsafe {
            ptr.write(value);
        }

        Ok(StreamingArenaRef { ptr, _phantom: PhantomData })
    }

    /// Allocate a slice
    pub fn alloc_slice<U>(&self, slice: &[U]) -> Result<StreamingArenaRef<[U]>, MemoryError>
    where U: Copy {
        if slice.is_empty() {
            return Ok(StreamingArenaRef {
                ptr: slice as *const [U] as *mut [U],
                _phantom: PhantomData,
            });
        }

        let size = std::mem::size_of::<U>() * slice.len();
        let align = std::mem::align_of::<U>();

        let ptr = self.alloc_bytes(size, align)? as *mut U;

        unsafe {
            std::ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            let slice_ptr = std::slice::from_raw_parts_mut(ptr, slice.len());

            Ok(StreamingArenaRef { ptr: slice_ptr as *mut [U], _phantom: PhantomData })
        }
    }

    /// Mark current position for later reset
    pub fn checkpoint(&self) -> StreamCheckpoint {
        StreamCheckpoint {
            buffer_count: self.active_buffers.borrow().len(),
            current_buffer: *self.current_buffer.borrow(),
            buffer_used: self
                .current_buffer
                .borrow()
                .and_then(|idx| self.active_buffers.borrow().get(idx).map(|b| b.used.get())),
        }
    }

    /// Reset to a previous checkpoint
    pub fn reset_to(&self, checkpoint: &StreamCheckpoint) {
        let mut buffers = self.active_buffers.borrow_mut();

        // Remove buffers allocated after checkpoint
        while buffers.len() > checkpoint.buffer_count {
            if let Some(buffer) = buffers.pop_back() {
                if self.options.recycle_buffers {
                    self.free_buffers.borrow_mut().push(buffer);
                }
            }
        }

        // Reset current buffer position
        *self.current_buffer.borrow_mut() = checkpoint.current_buffer;

        // Reset buffer usage
        if let Some(idx) = checkpoint.current_buffer {
            if let Some(used) = checkpoint.buffer_used {
                if let Some(buffer) = buffers.get(idx) {
                    buffer.used.set(used);
                }
            }
        }
    }

    /// Reset the entire arena
    pub fn reset(&mut self) {
        if self.options.recycle_buffers {
            // Move all active buffers to free list
            let mut active = self.active_buffers.borrow_mut();
            let mut free = self.free_buffers.borrow_mut();

            while let Some(buffer) = active.pop_front() {
                free.push(buffer);
            }
        } else {
            // Clear all buffers
            self.active_buffers.borrow_mut().clear();
        }

        *self.current_buffer.borrow_mut() = None;

        // Update stats
        if self.options.track_stats {
            self.stats.record_reset(0);
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &ArenaStats {
        &self.stats
    }
}

/// A checkpoint in the streaming arena
#[derive(Debug, Clone)]
pub struct StreamCheckpoint {
    buffer_count: usize,
    current_buffer: Option<usize>,
    buffer_used: Option<usize>,
}

/// A reference to a value in the streaming arena
pub struct StreamingArenaRef<T: ?Sized> {
    ptr: *mut T,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> StreamingArenaRef<T> {
    /// Get a reference to the value
    pub fn get(&self) -> &T {
        unsafe { &*self.ptr }
    }

    /// Get a mutable reference to the value
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }
}

impl<T> std::ops::Deref for StreamingArenaRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> std::ops::DerefMut for StreamingArenaRef<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let arena: StreamingArena<u32> = StreamingArena::default();

        let x = arena.alloc(42).unwrap();
        assert_eq!(*x, 42);

        let y = arena.alloc(100).unwrap();
        assert_eq!(*y, 100);
    }

    #[test]
    fn test_buffer_cycling() {
        let options = StreamOptions {
            buffer_size: 100,
            max_buffers: 2,
            recycle_buffers: true,
            ..Default::default()
        };

        let arena: StreamingArena<u64> = StreamingArena::new(options);

        // Allocate enough to fill multiple buffers
        let mut values = Vec::new();
        for i in 0..20 {
            values.push(arena.alloc(i).unwrap());
        }

        // Verify values
        for (i, value) in values.iter().enumerate() {
            assert_eq!(**value, i as u64);
        }

        // Should have recycled buffers
        assert!(arena.active_buffers.borrow().len() <= 2);
    }

    #[test]
    fn test_checkpoint_reset() {
        let arena: StreamingArena<String> = StreamingArena::default();

        let _s1 = arena.alloc("first".to_string()).unwrap();
        let checkpoint = arena.checkpoint();

        let _s2 = arena.alloc("second".to_string()).unwrap();
        let _s3 = arena.alloc("third".to_string()).unwrap();

        arena.reset_to(&checkpoint);

        // Can allocate again from checkpoint position
        let s4 = arena.alloc("fourth".to_string()).unwrap();
        assert_eq!(&**s4, "fourth");
    }

    #[test]
    fn test_slice_allocation() {
        let arena: StreamingArena = StreamingArena::default();

        let data = vec![1, 2, 3, 4, 5];
        let slice = arena.alloc_slice(&data).unwrap();

        assert_eq!(slice.get(), &data[..]);
    }

    #[test]
    fn test_large_allocation_fails() {
        let options = StreamOptions { buffer_size: 100, ..Default::default() };

        let arena: StreamingArena = StreamingArena::new(options);

        // Try to allocate more than buffer size
        let result = arena.alloc_bytes(200, 1);
        assert!(matches!(result, Err(MemoryError::allocation_too_large(0))));
    }
}
