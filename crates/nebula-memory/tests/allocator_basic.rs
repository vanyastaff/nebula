//! Basic integration tests for allocators
//!
//! Tests fundamental allocator functionality without complex dependencies

use nebula_memory::allocator::{Allocator, BumpAllocator};
use nebula_memory::core::traits::Resettable;
use std::alloc::Layout;

#[test]
fn test_bump_allocator_basic() {
    let allocator = BumpAllocator::new(4096).expect("Failed to create allocator");

    unsafe {
        // Allocate 64 bytes
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Allocation failed");

        // Write to allocated memory
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0x42, 64);

        // Verify we can read back
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0x42);

        // Deallocate
        allocator.deallocate(ptr.cast(), layout);
    }
}

#[test]
fn test_bump_allocator_multiple_allocations() {
    let allocator = BumpAllocator::new(4096).expect("Failed to create allocator");

    unsafe {
        let layout = Layout::from_size_align(32, 8).unwrap();

        // Make multiple allocations
        let ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");
        let ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");
        let ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");

        // Pointers should be different
        assert_ne!(ptr1.cast::<u8>().as_ptr(), ptr2.cast::<u8>().as_ptr());
        assert_ne!(ptr2.cast::<u8>().as_ptr(), ptr3.cast::<u8>().as_ptr());
        assert_ne!(ptr1.cast::<u8>().as_ptr(), ptr3.cast::<u8>().as_ptr());

        // Write different patterns to each
        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 0xAA, 32);
        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 0xBB, 32);
        std::ptr::write_bytes(ptr3.cast::<u8>().as_ptr(), 0xCC, 32);

        // Verify patterns
        assert_eq!(*ptr1.cast::<u8>().as_ptr(), 0xAA);
        assert_eq!(*ptr2.cast::<u8>().as_ptr(), 0xBB);
        assert_eq!(*ptr3.cast::<u8>().as_ptr(), 0xCC);

        // Cleanup
        allocator.deallocate(ptr1.cast(), layout);
        allocator.deallocate(ptr2.cast(), layout);
        allocator.deallocate(ptr3.cast(), layout);
    }
}

#[test]
fn test_bump_allocator_reset() {
    let allocator = BumpAllocator::new(4096).expect("Failed to create allocator");

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        // First allocation
        let ptr1 = allocator.allocate(layout).expect("First allocation failed");
        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 0x11, 128);

        // Reset allocator
        unsafe {
            allocator.reset();
        }

        // Second allocation after reset - should reuse space
        let ptr2 = allocator
            .allocate(layout)
            .expect("Second allocation failed");

        // After reset, the allocator reuses memory from the beginning
        // So ptr2 might equal ptr1 (implementation-dependent)

        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 0x22, 128);
        assert_eq!(*ptr2.cast::<u8>().as_ptr(), 0x22);

        allocator.deallocate(ptr2.cast(), layout);
    }
}

#[test]
fn test_bump_allocator_alignment() {
    let allocator = BumpAllocator::new(4096).expect("Failed to create allocator");

    unsafe {
        // Test different alignments
        let layout_8 = Layout::from_size_align(64, 8).unwrap();
        let layout_16 = Layout::from_size_align(64, 16).unwrap();
        let layout_32 = Layout::from_size_align(64, 32).unwrap();

        let ptr_8 = allocator
            .allocate(layout_8)
            .expect("8-byte alignment failed");
        let ptr_16 = allocator
            .allocate(layout_16)
            .expect("16-byte alignment failed");
        let ptr_32 = allocator
            .allocate(layout_32)
            .expect("32-byte alignment failed");

        // Check alignments
        assert_eq!(ptr_8.cast::<u8>().as_ptr() as usize % 8, 0);
        assert_eq!(ptr_16.cast::<u8>().as_ptr() as usize % 16, 0);
        assert_eq!(ptr_32.cast::<u8>().as_ptr() as usize % 32, 0);

        allocator.deallocate(ptr_8.cast(), layout_8);
        allocator.deallocate(ptr_16.cast(), layout_16);
        allocator.deallocate(ptr_32.cast(), layout_32);
    }
}

#[test]
fn test_bump_allocator_large_allocation() {
    let allocator = BumpAllocator::new(1024 * 1024).expect("Failed to create allocator");

    unsafe {
        // Allocate 256KB
        let layout = Layout::from_size_align(256 * 1024, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Large allocation failed");

        // Write and verify a pattern
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0xFF, 256 * 1024);
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0xFF);
        assert_eq!(*ptr.cast::<u8>().as_ptr().add(256 * 1024 - 1), 0xFF);

        allocator.deallocate(ptr.cast(), layout);
    }
}

#[test]
#[should_panic(expected = "Allocation failed")]
fn test_bump_allocator_out_of_memory() {
    let allocator = BumpAllocator::new(128).expect("Failed to create allocator");

    unsafe {
        // Try to allocate more than capacity
        let layout = Layout::from_size_align(256, 8).unwrap();
        let _ = allocator.allocate(layout).expect("Allocation failed");
    }
}

#[test]
fn test_bump_allocator_zero_sized() {
    let allocator = BumpAllocator::new(4096).expect("Failed to create allocator");

    unsafe {
        // Zero-sized allocation
        let layout = Layout::from_size_align(0, 1).unwrap();
        let result = allocator.allocate(layout);

        // Zero-sized allocations should succeed (or fail gracefully)
        match result {
            Ok(ptr) => {
                // If it succeeds, deallocate
                allocator.deallocate(ptr.cast(), layout);
            }
            Err(_) => {
                // Zero-sized allocations may be rejected - this is also valid
            }
        }
    }
}

#[test]
fn test_bump_allocator_concurrent_allocations() {
    use std::sync::Arc;
    use std::thread;

    let allocator = Arc::new(BumpAllocator::new(1024 * 1024).expect("Failed to create allocator"));
    let mut handles = vec![];

    // Spawn 10 threads, each making allocations
    for i in 0..10 {
        let allocator_clone = Arc::clone(&allocator);
        let handle = thread::spawn(move || {
            unsafe {
                let layout = Layout::from_size_align(128, 8).unwrap();
                for _ in 0..10 {
                    if let Ok(ptr) = allocator_clone.allocate(layout) {
                        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), i as u8, 128);
                        // Note: We don't deallocate in bump allocator typically
                        allocator_clone.deallocate(ptr.cast(), layout);
                    }
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}
