//! Stack allocator implementation
//!
//! A stack allocator allocates memory in a LIFO (Last In, First Out) manner,
//! similar to how a call stack works. Unlike a bump allocator, it supports
//! deallocating the most recently allocated block, allowing for stack-like
//! memory management patterns.
//!
//! # Use Cases
//! - Nested scoping allocations
//! - Recursive algorithms with temporary storage
//! - Expression evaluation with temporary results
//! - Function call simulation with local variables
//! - Any scenario requiring LIFO allocation/deallocation

use core::alloc::Layout;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};

use super::{AllocError, AllocErrorCode, AllocResult, Allocator, MemoryUsage, Resettable};

/// Stack allocator that supports LIFO allocation and deallocation
///
/// This allocator maintains a stack-like structure where memory can only
/// be deallocated in reverse order of allocation. It's more flexible than
/// a bump allocator but still very efficient.
///
/// # Memory Layout
/// ```text
/// [start]----[alloc1]----[alloc2]----[alloc3]----[top]----[free]----[end]
///             <------ allocated ------>         <-- available -->
/// ```
///
/// Deallocations must happen in reverse order: alloc3, then alloc2, then
/// alloc1.
pub struct StackAllocator {
    /// Owned memory buffer
    memory: Box<[u8]>,

    /// Start of the memory region (cached for performance)
    start_addr: usize,

    /// Current top of stack (atomic for thread safety)
    top: AtomicUsize,

    /// End address (cached for performance)
    end_addr: usize,
}

/// Stack frame marker for tracking allocations
///
/// This marker allows for scoped deallocation - you can save a marker,
/// make several allocations, then restore to the marker to deallocate
/// all allocations made after the marker was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackMarker {
    position: usize,
}

impl StackAllocator {
    /// Creates a new stack allocator with the specified capacity
    pub fn new(capacity: usize) -> AllocResult<Self> {
        if capacity == 0 {
            return Err(AllocError::with_layout(
                AllocErrorCode::InvalidLayout,
                Layout::from_size_align(0, 1).unwrap(),
            ));
        }

        let memory = vec![0u8; capacity].into_boxed_slice();
        let start_addr = memory.as_ptr() as usize;
        let end_addr = start_addr + capacity;

        Ok(Self { memory, start_addr, top: AtomicUsize::new(start_addr), end_addr })
    }

    /// Creates a stack allocator from a pre-allocated boxed slice
    pub fn from_boxed_slice(memory: Box<[u8]>) -> Self {
        let start_addr = memory.as_ptr() as usize;
        let end_addr = start_addr + memory.len();

        Self { memory, start_addr, top: AtomicUsize::new(start_addr), end_addr }
    }

    /// Returns the total capacity of the allocator
    pub fn capacity(&self) -> usize {
        self.memory.len()
    }

    /// Returns the amount of memory currently allocated
    pub fn used(&self) -> usize {
        let top = self.top.load(Ordering::Acquire);
        top.saturating_sub(self.start_addr)
    }

    /// Returns the amount of memory available for allocation
    pub fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }

    /// Returns the current top of stack position
    pub fn current_top(&self) -> usize {
        self.top.load(Ordering::Acquire)
    }

    /// Creates a marker at the current stack position
    ///
    /// This marker can be used later to restore the stack to this position,
    /// effectively deallocating all allocations made after this point.
    pub fn mark(&self) -> StackMarker {
        StackMarker { position: self.top.load(Ordering::Acquire) }
    }

    /// Restores the stack to a previous marker position
    ///
    /// This deallocates all allocations made after the marker was created.
    ///
    /// # Safety
    /// - The marker must be valid (created by this allocator)
    /// - All pointers to memory allocated after the marker become invalid
    /// - The marker position must not be in the future (greater than current
    ///   top)
    pub unsafe fn restore_to_marker(&self, marker: StackMarker) -> Result<(), AllocError> {
        let current_top = self.top.load(Ordering::Acquire);

        if marker.position > current_top {
            return Err(AllocError::invalid_layout()); // marker from the future
        }
        if marker.position < self.start_addr || marker.position > self.end_addr {
            return Err(AllocError::invalid_layout()); // out of bounds
        }

        self.top.store(marker.position, Ordering::Release);
        Ok(())
    }

    /// Pops the most recent allocation if it matches the given pointer and
    /// layout
    ///
    /// This provides a safe way to deallocate the most recent allocation.
    /// Returns true if the deallocation was successful, false if the pointer
    /// doesn't match the most recent allocation.
    ///
    /// # Safety
    /// - The pointer must have been allocated by this allocator
    /// - The layout must match the original allocation layout
    pub unsafe fn try_pop(&self, ptr: NonNull<u8>, layout: Layout) -> bool {
        let current_top = self.top.load(Ordering::Acquire);
        let expected_start = current_top.saturating_sub(layout.size());

        // Check if this pointer matches the most recent allocation
        if ptr.as_ptr() as usize == Self::align_up(expected_start, layout.align()) {
            // This is the most recent allocation, we can safely pop it
            self.top.store(expected_start, Ordering::Release);
            true
        } else {
            // Not the most recent allocation, cannot pop
            false
        }
    }

    /// Aligns a size up to the specified alignment
    #[inline]
    fn align_up(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
    }

    /// Maximum backoff iterations
    const MAX_BACKOFF: usize = 32; // Smaller than bump allocator since contention is less likely
    const MAX_RETRIES: usize = 500;

    /// Attempts to allocate memory with adaptive backoff
    fn try_allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let mut backoff = 0;
        let mut attempts = 0;

        loop {
            let current_top = self.top.load(Ordering::Acquire);
            let aligned_addr = Self::align_up(current_top, align);
            let new_top = aligned_addr.checked_add(size)?;

            // Check if we have enough space
            if new_top > self.end_addr {
                return None;
            }

            // Try to update the top atomically
            let result = if attempts == 0 {
                self.top.compare_exchange(current_top, new_top, Ordering::AcqRel, Ordering::Acquire)
            } else {
                self.top.compare_exchange_weak(
                    current_top,
                    new_top,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            };

            match result {
                Ok(_) => return Some(unsafe { NonNull::new_unchecked(aligned_addr as *mut u8) }),
                Err(_) => {
                    attempts += 1;

                    if attempts >= Self::MAX_RETRIES {
                        return None;
                    }

                    // Exponential backoff
                    if backoff > 0 {
                        for _ in 0..backoff {
                            core::hint::spin_loop();
                        }
                    }

                    backoff = if backoff == 0 { 1 } else { (backoff * 2).min(Self::MAX_BACKOFF) };
                },
            }
        }
    }

    /// Convenience constructors
    pub fn small() -> AllocResult<Self> {
        Self::new(32 * 1024)
    } // 32KB
    pub fn medium() -> AllocResult<Self> {
        Self::new(512 * 1024)
    } // 512KB
    pub fn large() -> AllocResult<Self> {
        Self::new(8 * 1024 * 1024)
    } // 8MB
}

unsafe impl Allocator for StackAllocator {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        if layout.size() == 0 {
            // Handle zero-sized allocations
            let ptr = NonNull::<u8>::dangling();
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }

        if let Some(ptr) = self.try_allocate(layout.size(), layout.align()) {
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            Err(AllocError::with_layout(AllocErrorCode::OutOfMemory, layout))
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Try to pop if this is the most recent allocation
        // This is a "best effort" - if it's not the most recent, it's a no-op
        unsafe { self.try_pop(ptr, layout) };
    }

    unsafe fn reallocate(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> AllocResult<NonNull<[u8]>> {
        // Check if this is the most recent allocation and we can extend in-place
        let current_top = self.top.load(Ordering::Acquire);
        let ptr_addr = ptr.as_ptr() as usize;
        let expected_start = current_top.saturating_sub(old_layout.size());

        if ptr_addr == Self::align_up(expected_start, old_layout.align())
            && new_layout.align() <= old_layout.align()
            && new_layout.size() >= old_layout.size()
        {
            // This is the most recent allocation, try to extend in-place
            let additional_size = new_layout.size() - old_layout.size();
            let new_top = current_top + additional_size;

            if new_top <= self.end_addr {
                // We have space to extend
                if self
                    .top
                    .compare_exchange(current_top, new_top, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
                }
            }
        }

        // Fall back to allocate + copy + deallocate
        let new_ptr = unsafe { self.allocate(new_layout)? };

        let copy_size = core::cmp::min(old_layout.size(), new_layout.size());
        if copy_size > 0 {
            unsafe {
                ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr() as *mut u8, copy_size);
            }
        }

        // Try to deallocate old memory (will succeed if it's still the most recent)
        unsafe { self.deallocate(ptr, old_layout) };

        Ok(new_ptr)
    }
}

impl MemoryUsage for StackAllocator {
    fn used_memory(&self) -> usize {
        self.used()
    }

    fn available_memory(&self) -> Option<usize> {
        Some(self.available())
    }

    fn total_memory(&self) -> Option<usize> {
        Some(self.capacity())
    }
}

impl Resettable for StackAllocator {
    unsafe fn reset(&self) {
        self.top.store(self.start_addr, Ordering::Release);
    }

    fn can_reset(&self) -> bool {
        true
    }
}

// Thread safety
unsafe impl Send for StackAllocator {}
unsafe impl Sync for StackAllocator {}

/// RAII helper for automatic stack restoration
///
/// This struct automatically restores the stack to a marked position
/// when it goes out of scope, providing exception-safe stack management.
pub struct StackFrame<'a> {
    allocator: &'a StackAllocator,
    marker: StackMarker,
}

impl<'a> StackFrame<'a> {
    /// Creates a new stack frame that will restore to the current position
    /// when dropped
    pub fn new(allocator: &'a StackAllocator) -> Self {
        let marker = allocator.mark();
        Self { allocator, marker }
    }

    /// Gets the underlying allocator
    pub fn allocator(&self) -> &'a StackAllocator {
        self.allocator
    }

    /// Manually restore and consume this frame
    pub fn restore(self) {
        // Drop will handle the restoration
        drop(self);
    }
}

impl<'a> Drop for StackFrame<'a> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.allocator.restore_to_marker(self.marker);
        }
    }
}

/// Convenience macro for stack frame allocation
#[macro_export]
macro_rules! with_stack_frame {
    ($allocator:expr, $body:block) => {{
        let _frame = StackFrame::new($allocator);
        $body
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            let ptr2 = allocator.allocate(layout).unwrap();

            assert_eq!(ptr1.len(), 8);
            assert_eq!(ptr2.len(), 8);
            assert_ne!(ptr1.as_ptr(), ptr2.as_ptr());
        }

        assert!(allocator.used() >= 16);
    }

    #[test]
    fn test_stack_deallocation() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            let used_after_first = allocator.used();

            let ptr2 = allocator.allocate(layout).unwrap();
            let used_after_second = allocator.used();

            // Deallocate the most recent allocation (ptr2)
            allocator.deallocate(ptr2.cast(), layout);
            assert_eq!(allocator.used(), used_after_first);

            // Deallocate the first allocation (ptr1)
            allocator.deallocate(ptr1.cast(), layout);
            assert_eq!(allocator.used(), 0);
        }
    }

    #[test]
    fn test_stack_marker() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u32>();

        unsafe {
            let _ = allocator.allocate(layout).unwrap();
            let marker = allocator.mark();

            let _ = allocator.allocate(layout).unwrap();
            let _ = allocator.allocate(layout).unwrap();

            let used_before_restore = allocator.used();
            assert!(used_before_restore >= 12); // At least 3 * 4 bytes

            allocator.restore_to_marker(marker).unwrap();
            assert!(allocator.used() < used_before_restore);
        }
    }

    #[test]
    fn test_stack_frame_raii() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u32>();

        unsafe {
            let _ = allocator.allocate(layout).unwrap();
            let used_before_frame = allocator.used();

            {
                let _frame = StackFrame::new(&allocator);
                let _ = allocator.allocate(layout).unwrap();
                let _ = allocator.allocate(layout).unwrap();

                assert!(allocator.used() > used_before_frame);
            } // _frame is dropped here, automatically restoring

            assert_eq!(allocator.used(), used_before_frame);
        }
    }

    #[test]
    fn test_out_of_order_deallocation() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            let ptr2 = allocator.allocate(layout).unwrap();
            let used_before = allocator.used();

            // Try to deallocate ptr1 (not the most recent) - should be no-op
            allocator.deallocate(ptr1.cast(), layout);
            assert_eq!(allocator.used(), used_before); // No change

            // Deallocate ptr2 (most recent) - should succeed
            allocator.deallocate(ptr2.cast(), layout);
            assert!(allocator.used() < used_before);
        }
    }

    #[test]
    fn test_reallocate_in_place() {
        let allocator = StackAllocator::new(1024).unwrap();

        unsafe {
            let old_layout = Layout::from_size_align(8, 8).unwrap();
            let ptr = allocator.allocate(old_layout).unwrap();

            // Write test data
            (ptr.as_ptr() as *mut u64).write(0xDEADBEEF);

            let used_before = allocator.used();

            // Reallocate to larger size
            let new_layout = Layout::from_size_align(16, 8).unwrap();
            let new_ptr = allocator.reallocate(ptr.cast(), old_layout, new_layout).unwrap();

            // Data should be preserved regardless of whether it's in-place or not
            assert_eq!((new_ptr.as_ptr() as *const u64).read(), 0xDEADBEEF);
            assert_eq!(new_ptr.len(), 16);

            // Memory usage should have increased
            assert!(allocator.used() > used_before);
        }
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let allocator = Arc::new(StackAllocator::new(4096).unwrap());
        let layout = Layout::new::<u64>();

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let alloc = allocator.clone();
                thread::spawn(move || {
                    for _ in 0..10 {
                        unsafe {
                            if let Ok(ptr) = alloc.allocate(layout) {
                                // Write some data
                                (ptr.as_ptr() as *mut u64).write(42);
                                // In a real scenario, we'd need coordination
                                // for proper deallocation
                            }
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert!(allocator.used() > 0);
    }

    #[test]
    fn test_macro() {
        let allocator = StackAllocator::new(1024).unwrap();
        let layout = Layout::new::<u32>();

        unsafe {
            let _ = allocator.allocate(layout).unwrap();
            let used_before = allocator.used();

            with_stack_frame!(&allocator, {
                let _ = allocator.allocate(layout).unwrap();
                let _ = allocator.allocate(layout).unwrap();
                assert!(allocator.used() > used_before);
            });

            assert_eq!(allocator.used(), used_before);
        }
    }

    #[test]
    fn test_stats() {
        let allocator = StackAllocator::medium().unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            let _ = allocator.allocate(layout).unwrap();
        }

        // Just check basic metrics through MemoryUsage trait
        assert!(allocator.used_memory() > 0);
        assert!(allocator.available_memory().unwrap() < allocator.total_memory().unwrap());
    }
}
