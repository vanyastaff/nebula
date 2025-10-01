//! Integration tests for Stack allocator

use nebula_memory::allocator::{Allocator, StackAllocator, StackConfig, StackMarker};
use std::alloc::Layout;

#[test]
fn test_stack_allocator_basic() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Allocation failed");

        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0x55, 128);
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0x55);

        allocator.deallocate(ptr.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_lifo() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Allocate in order: A, B, C
        let ptr_a = allocator.allocate(layout).expect("Allocation A failed");
        let ptr_b = allocator.allocate(layout).expect("Allocation B failed");
        let ptr_c = allocator.allocate(layout).expect("Allocation C failed");

        std::ptr::write_bytes(ptr_a.cast::<u8>().as_ptr(), 0xAA, 64);
        std::ptr::write_bytes(ptr_b.cast::<u8>().as_ptr(), 0xBB, 64);
        std::ptr::write_bytes(ptr_c.cast::<u8>().as_ptr(), 0xCC, 64);

        // Verify
        assert_eq!(*ptr_a.cast::<u8>().as_ptr(), 0xAA);
        assert_eq!(*ptr_b.cast::<u8>().as_ptr(), 0xBB);
        assert_eq!(*ptr_c.cast::<u8>().as_ptr(), 0xCC);

        // Deallocate in LIFO order: C, B, A
        allocator.deallocate(ptr_c.cast(), layout);
        allocator.deallocate(ptr_b.cast(), layout);
        allocator.deallocate(ptr_a.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_marker() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Get initial marker
        let marker = allocator.mark();

        // Make some allocations
        let ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");
        let ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");

        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 0x11, 64);
        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 0x22, 64);

        // Release to marker - should free both allocations
        allocator.release(marker);

        // Allocate again - should reuse space
        let ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");
        std::ptr::write_bytes(ptr3.cast::<u8>().as_ptr(), 0x33, 64);
        assert_eq!(*ptr3.cast::<u8>().as_ptr(), 0x33);

        allocator.deallocate(ptr3.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_nested_markers() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(32, 8).unwrap();

        // Outer scope
        let marker1 = allocator.mark();
        let _ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");

        // Middle scope
        let marker2 = allocator.mark();
        let _ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");

        // Inner scope
        let marker3 = allocator.mark();
        let _ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");

        // Release inner scope
        allocator.release(marker3);

        // Release middle scope
        allocator.release(marker2);

        // Release outer scope
        allocator.release(marker1);
    }
}

#[test]
fn test_stack_allocator_reset() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        // Make allocations
        let ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");
        let _ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");

        let addr1 = ptr1.cast::<u8>().as_ptr() as usize;

        // Reset
        unsafe { allocator.reset(); };

        // Allocate after reset
        let ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");
        let addr3 = ptr3.cast::<u8>().as_ptr() as usize;

        // Should reuse from beginning
        assert_eq!(addr1, addr3);

        allocator.deallocate(ptr3.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_alignment() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout_8 = Layout::from_size_align(64, 8).unwrap();
        let layout_16 = Layout::from_size_align(64, 16).unwrap();
        let layout_32 = Layout::from_size_align(64, 32).unwrap();

        let ptr_8 = allocator.allocate(layout_8).expect("8-byte alignment failed");
        let ptr_16 = allocator.allocate(layout_16).expect("16-byte alignment failed");
        let ptr_32 = allocator.allocate(layout_32).expect("32-byte alignment failed");

        assert_eq!(ptr_8.cast::<u8>().as_ptr() as usize % 8, 0);
        assert_eq!(ptr_16.cast::<u8>().as_ptr() as usize % 16, 0);
        assert_eq!(ptr_32.cast::<u8>().as_ptr() as usize % 32, 0);

        allocator.deallocate(ptr_32.cast(), layout_32);
        allocator.deallocate(ptr_16.cast(), layout_16);
        allocator.deallocate(ptr_8.cast(), layout_8);
    }
}

#[test]
fn test_stack_allocator_frame() {
    use nebula_memory::allocator::StackFrame;

    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Create frame (RAII style)
        {
            let _frame = StackFrame::new(&allocator);
            let _ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");
            let _ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");
            // Frame will release on drop
        }

        // After frame drop, allocations should be freed
        let ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");
        allocator.deallocate(ptr3.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_multiple_frames() {
    use nebula_memory::allocator::StackFrame;

    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(4096, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(32, 8).unwrap();

        // Outer frame
        let _frame1 = StackFrame::new(&allocator);
        let _ptr1 = allocator.allocate(layout).expect("Allocation 1 failed");

        {
            // Middle frame
            let _frame2 = StackFrame::new(&allocator);
            let _ptr2 = allocator.allocate(layout).expect("Allocation 2 failed");

            {
                // Inner frame
                let _frame3 = StackFrame::new(&allocator);
                let _ptr3 = allocator.allocate(layout).expect("Allocation 3 failed");
                // Inner frame releases here
            }

            // Middle frame releases here
        }

        // Outer frame still active
        let _ptr4 = allocator.allocate(layout).expect("Allocation 4 failed");
        // Outer frame releases at end of function
    }
}

#[test]
fn test_stack_allocator_large_allocation() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(1024 * 1024, config)
        .expect("Failed to create stack allocator");

    unsafe {
        // Allocate 512KB
        let layout = Layout::from_size_align(512 * 1024, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Large allocation failed");

        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0xEE, 512 * 1024);
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0xEE);
        assert_eq!(*ptr.cast::<u8>().as_ptr().add(512 * 1024 - 1), 0xEE);

        allocator.deallocate(ptr.cast(), layout);
    }
}

#[test]
fn test_stack_allocator_stress() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(64 * 1024, config)
        .expect("Failed to create stack allocator");

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        // Repeatedly allocate and release with markers
        for _ in 0..100 {
            let marker = allocator.mark();

            for i in 0..10 {
                let ptr = allocator.allocate(layout).expect("Allocation failed");
                std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), i as u8, 128);
            }

            allocator.release(marker);
        }
    }
}
