//! Production-ready bump allocator with safe abstractions
//!
//! A bump allocator (also called arena allocator) provides fast sequential allocations
//! by simply incrementing a pointer. All memory is freed at once when the allocator is dropped.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec, vec::Vec};

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

mod checkpoint;
mod config;
mod cursor;

pub use checkpoint::{BumpCheckpoint, BumpScope};
pub use config::BumpConfig;
use cursor::{AtomicCursor, CellCursor, Cursor};

use crate::allocator::{
    AllocError, AllocResult, Allocator, BulkAllocator, MemoryUsage, OptionalStats, Resettable,
    StatisticsProvider, ThreadSafeAllocator,
};

use crate::utils::{Backoff, MemoryOps, PrefetchManager, align_up, atomic_max, cache_line_size};

/// Thread-safe wrapper for memory buffer with interior mutability
#[repr(transparent)]
struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

// SAFETY: We ensure proper synchronization through atomic cursor
unsafe impl<T: ?Sized> Sync for SyncUnsafeCell<T> {}
unsafe impl<T: ?Sized + Send> Send for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
}

impl<T: ?Sized> SyncUnsafeCell<T> {
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// Production-ready bump allocator
pub struct BumpAllocator {
    /// Memory buffer with explicit interior mutability for safe mutable access
    memory: Box<SyncUnsafeCell<[u8]>>,
    config: BumpConfig,
    prefetch_mgr: PrefetchManager,
    memory_ops: MemoryOps,
    start_addr: usize,
    end_addr: usize,
    capacity: usize,
    cursor: Box<dyn Cursor>,
    stats: OptionalStats,
    peak_usage: AtomicUsize,
    generation: AtomicU32,
}

impl BumpAllocator {
    /// Creates a new bump allocator with specified capacity and configuration
    pub fn with_config(capacity: usize, config: BumpConfig) -> AllocResult<Self> {
        if capacity == 0 {
            return Err(AllocError::invalid_layout());
        }

        let mut vec = vec![0u8; capacity];

        let memory_ops = MemoryOps::new();
        if let Some(pattern) = config.alloc_pattern {
            unsafe {
                MemoryOps::secure_fill_slice(&mut vec, pattern);
            }
        }

        // Wrap in SyncUnsafeCell for interior mutability
        // We need to convert Box<[u8]> into Box<SyncUnsafeCell<[u8]>>
        let boxed_slice = vec.into_boxed_slice();
        let len = boxed_slice.len();
        let ptr = Box::into_raw(boxed_slice) as *mut u8;
        // SAFETY: We're transmuting Box<[u8]> layout to Box<SyncUnsafeCell<[u8]>> layout
        // This is safe because SyncUnsafeCell is repr(transparent)
        let memory: Box<SyncUnsafeCell<[u8]>> = unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut SyncUnsafeCell<[u8]>)
        };

        // SAFETY: We just created the memory, getting pointer is safe
        let start_addr = unsafe { (*memory.get()).as_ptr() as usize };
        let end_addr = start_addr + capacity;

        let cursor: Box<dyn Cursor> = if config.thread_safe {
            Box::new(AtomicCursor::new(start_addr))
        } else {
            Box::new(CellCursor::new(start_addr))
        };

        let track_stats = config.track_stats;

        Ok(Self {
            memory,
            config,
            prefetch_mgr: PrefetchManager::new(),
            memory_ops,
            start_addr,
            end_addr,
            capacity,
            cursor,
            stats: if track_stats {
                OptionalStats::enabled()
            } else {
                OptionalStats::disabled()
            },
            peak_usage: AtomicUsize::new(0),
            generation: AtomicU32::new(0),
        })
    }

    /// Creates allocator with default configuration
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::default())
    }

    /// Creates production-optimized allocator
    pub fn production(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::production())
    }

    /// Creates debug-optimized allocator
    pub fn debug(capacity: usize) -> AllocResult<Self> {
        Self::with_config(capacity, BumpConfig::debug())
    }

    /// Convenience: 64KB allocator
    pub fn small() -> AllocResult<Self> {
        Self::production(64 * 1024)
    }

    /// Convenience: 1MB allocator
    pub fn medium() -> AllocResult<Self> {
        Self::production(1024 * 1024)
    }

    /// Convenience: 16MB allocator
    pub fn large() -> AllocResult<Self> {
        Self::production(16 * 1024 * 1024)
    }

    /// Total capacity
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Currently used memory
    #[inline]
    pub fn used(&self) -> usize {
        self.cursor
            .load(Ordering::Relaxed)
            .saturating_sub(self.start_addr)
    }

    /// Available memory
    #[inline]
    pub fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }

    /// Peak usage
    #[inline]
    pub fn peak_usage(&self) -> usize {
        self.peak_usage.load(Ordering::Relaxed)
    }

    /// Creates a checkpoint at current position
    #[must_use = "checkpoint должен быть сохранён для последующего restore"]
    pub fn checkpoint(&self) -> BumpCheckpoint {
        BumpCheckpoint {
            position: self.cursor.load(Ordering::Acquire),
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    /// Restores allocator to previous checkpoint
    pub fn restore(&self, checkpoint: BumpCheckpoint) -> AllocResult<()> {
        let current_gen = self.generation.load(Ordering::Acquire);
        if checkpoint.generation != current_gen {
            return Err(AllocError::invalid_input(
                "checkpoint from different generation",
            ));
        }

        let current = self.cursor.load(Ordering::Acquire);
        if checkpoint.position < self.start_addr || checkpoint.position > self.end_addr {
            return Err(AllocError::invalid_input(
                "checkpoint position out of bounds",
            ));
        }
        if checkpoint.position > current {
            return Err(AllocError::invalid_input("checkpoint is in the future"));
        }

        // Fill deallocated region with pattern
        if let Some(pattern) = self.config.dealloc_pattern {
            let start = checkpoint.position - self.start_addr;
            let end = current - self.start_addr;
            unsafe {
                // SAFETY: We have exclusive access through UnsafeCell
                let memory_slice = &mut *self.memory.get();
                if let Some(slice) = memory_slice.get_mut(start..end) {
                    MemoryOps::secure_fill_slice(slice, pattern);
                }
            }
        }

        self.cursor.store(checkpoint.position, Ordering::Release);
        Ok(())
    }

    /// Creates scoped allocation with auto-restore
    pub fn scoped(&self) -> BumpScope<'_> {
        BumpScope::new(self)
    }

    /// Resets allocator to initial state (internal helper)
    fn reset_internal(&self) {
        self.cursor.store(self.start_addr, Ordering::Release);
        self.generation.fetch_add(1, Ordering::Release);
        self.peak_usage.store(0, Ordering::Relaxed);
        self.stats.reset();
    }

    #[inline]
    fn effective_size(&self, size: usize) -> usize {
        size.max(self.config.min_alloc_size)
    }

    #[inline]
    fn prefetch_next(&self, addr: usize) {
        if !self.config.enable_prefetch {
            return;
        }
        let prefetch_dist = self.config.prefetch_distance * cache_line_size();
        let prefetch_end = (addr + prefetch_dist).min(self.end_addr);
        if addr < self.end_addr && prefetch_end > addr {
            let start = addr - self.start_addr;
            let end = prefetch_end - self.start_addr;
            unsafe {
                // SAFETY: Read-only access for prefetch
                let memory_slice = &*self.memory.get();
                if let Some(slice) = memory_slice.get(start..end) {
                    self.prefetch_mgr.prefetch_slice_read(slice);
                }
            }
        }
    }

    fn try_bump(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let actual_size = self.effective_size(size);
        const MAX_RETRIES: usize = 100;
        let mut backoff = Backoff::new();
        let mut attempts = 0;

        loop {
            if attempts >= MAX_RETRIES {
                self.stats.record_allocation_failure();
                return None;
            }

            let current = self.cursor.load(Ordering::Acquire);
            let aligned = align_up(current, align);
            let new_current = aligned.checked_add(actual_size)?;

            if new_current > self.end_addr {
                self.stats.record_allocation_failure();
                return None;
            }

            self.prefetch_next(new_current);

            let result = self.cursor.compare_exchange_weak(
                current,
                new_current,
                Ordering::AcqRel,
                Ordering::Acquire,
            );

            match result {
                Ok(_) => {
                    self.stats.record_allocation(actual_size);
                    let usage = new_current - self.start_addr;
                    atomic_max(&self.peak_usage, usage);

                    // Calculate return pointer with proper provenance through UnsafeCell
                    let offset = aligned - self.start_addr;
                    let ptr = unsafe {
                        // SAFETY: offset is within bounds (checked by compare_exchange)
                        // UnsafeCell grants us mutable access
                        let memory_ptr = self.memory.get();
                        (*memory_ptr).as_mut_ptr().add(offset)
                    };

                    // Fill with pattern if configured
                    if let Some(pattern) = self.config.alloc_pattern {
                        unsafe {
                            // SAFETY: We just allocated this memory, it's uninitialized
                            core::ptr::write_bytes(ptr, pattern, actual_size);
                        }
                    }

                    return Some(unsafe { NonNull::new_unchecked(ptr) });
                }
                Err(_) => {
                    attempts += 1;
                    if self.config.thread_safe {
                        backoff.spin();
                    }
                }
            }
        }
    }
}

unsafe impl Allocator for BumpAllocator {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        let ptr = self
            .try_bump(layout.size(), layout.align())
            .ok_or_else(|| AllocError::out_of_memory_with_layout(layout))?;

        let slice = NonNull::slice_from_raw_parts(ptr, layout.size());
        Ok(slice)
    }

    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        // Bump allocator doesn't support individual deallocation
    }
}

unsafe impl ThreadSafeAllocator for BumpAllocator {}

unsafe impl BulkAllocator for BumpAllocator {}

impl MemoryUsage for BumpAllocator {
    fn used_memory(&self) -> usize {
        self.used()
    }

    fn available_memory(&self) -> Option<usize> {
        Some(self.available())
    }
}

impl Resettable for BumpAllocator {
    unsafe fn reset(&self) {
        self.reset_internal()
    }
}

impl StatisticsProvider for BumpAllocator {
    fn statistics(&self) -> crate::allocator::AllocatorStats {
        self.stats.snapshot().unwrap_or_default()
    }

    fn reset_statistics(&self) {
        self.stats.reset()
    }
}
