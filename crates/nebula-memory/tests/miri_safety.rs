//! Miri safety tests for nebula-memory allocators
//!
//! These tests verify undefined behavior detection using Miri.
//! Run with: cargo +nightly miri test -p nebula-memory --test miri_safety

#![cfg(miri)]

use nebula_memory::allocator::bump::BumpConfig;
use nebula_memory::allocator::{
    Allocator, BumpAllocator, PoolAllocator, PoolConfig, StackAllocator, StackConfig,
};
use nebula_memory::core::traits::Resettable;
use std::alloc::Layout;

/// Test basic allocation and deallocation under Miri
#[test]
fn miri_bump_allocator_basic() {
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(4096, config).expect("Failed to create allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Allocation failed");

        // Write to allocated memory
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0x42, 64);

        // Read back
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0x42);

        // Deallocate
        allocator.deallocate(ptr.cast(), layout);
    }
}

/// Test pool allocator reuse under Miri
#[test]
fn miri_pool_allocator_reuse() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(128, 8, 16, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        // Allocate
        let ptr1 = allocator.allocate(layout).unwrap();
        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 0xFF, 128);

        // Deallocate
        allocator.deallocate(ptr1.cast(), layout);

        // Allocate again - should reuse
        let ptr2 = allocator.allocate(layout).unwrap();

        // Write new data
        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 0xAA, 128);

        // Verify
        assert_eq!(*ptr2.cast::<u8>().as_ptr(), 0xAA);

        allocator.deallocate(ptr2.cast(), layout);
    }
}

/// Test stack allocator LIFO ordering under Miri
#[test]
fn miri_stack_allocator_lifo() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(8192, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(256, 8).unwrap();

        // Allocate in order
        let ptr1 = allocator.allocate(layout).unwrap();
        let ptr2 = allocator.allocate(layout).unwrap();
        let ptr3 = allocator.allocate(layout).unwrap();

        // Write to each
        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 1, 256);
        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 2, 256);
        std::ptr::write_bytes(ptr3.cast::<u8>().as_ptr(), 3, 256);

        // Verify writes
        assert_eq!(*ptr1.cast::<u8>().as_ptr(), 1);
        assert_eq!(*ptr2.cast::<u8>().as_ptr(), 2);
        assert_eq!(*ptr3.cast::<u8>().as_ptr(), 3);

        // Deallocate in LIFO order
        allocator.deallocate(ptr3.cast(), layout);
        allocator.deallocate(ptr2.cast(), layout);
        allocator.deallocate(ptr1.cast(), layout);
    }
}

/// Test alignment requirements under Miri
#[test]
fn miri_alignment_safety() {
    // Disable alloc_pattern for Miri (strict provenance)
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(4096, config).unwrap();

    unsafe {
        // Test various alignments
        for align in [1, 2, 4, 8, 16, 32, 64].iter() {
            let layout = Layout::from_size_align(128, *align).unwrap();
            let ptr = allocator.allocate(layout).unwrap();

            // Verify alignment
            let addr = ptr.cast::<u8>().as_ptr() as usize;
            assert_eq!(addr % align, 0, "Pointer not aligned to {}", align);

            // Write to memory
            std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0xFF, 128);

            allocator.deallocate(ptr.cast(), layout);
        }
    }
}

/// Test reset doesn't cause use-after-free under Miri
#[test]
fn miri_reset_safety() {
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(4096, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Allocate and write
        let ptr1 = allocator.allocate(layout).unwrap();
        std::ptr::write_bytes(ptr1.cast::<u8>().as_ptr(), 0x11, 64);

        // Reset arena
        allocator.reset();

        // Allocate again - should get fresh memory
        let ptr2 = allocator.allocate(layout).unwrap();
        std::ptr::write_bytes(ptr2.cast::<u8>().as_ptr(), 0x22, 64);

        assert_eq!(*ptr2.cast::<u8>().as_ptr(), 0x22);

        allocator.deallocate(ptr2.cast(), layout);
    }
}

/// Test multiple allocations don't overlap under Miri
#[test]
fn miri_no_overlap() {
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(8192, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        let mut ptrs = Vec::new();

        // Allocate multiple blocks
        for i in 0..10 {
            let ptr = allocator.allocate(layout).unwrap();
            std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), i as u8, 128);
            ptrs.push(ptr);
        }

        // Verify each block has correct value
        for (i, ptr) in ptrs.iter().enumerate() {
            assert_eq!(*ptr.cast::<u8>().as_ptr(), i as u8);
        }

        // Cleanup
        for ptr in ptrs {
            allocator.deallocate(ptr.cast(), layout);
        }
    }
}

/// Test pool exhaustion handling under Miri
#[test]
fn miri_pool_exhaustion() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(64, 8, 4, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        let mut ptrs = Vec::new();

        // Allocate up to capacity
        for _ in 0..4 {
            let ptr = allocator.allocate(layout).unwrap();
            ptrs.push(ptr);
        }

        // Next allocation should fail
        let result = allocator.allocate(layout);
        assert!(result.is_err(), "Pool should be exhausted");

        // Cleanup
        for ptr in ptrs {
            allocator.deallocate(ptr.cast(), layout);
        }
    }
}

/// Test zero-sized allocations under Miri
#[test]
fn miri_zero_sized_allocations() {
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(4096, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(0, 1).unwrap();

        // Zero-sized allocations should succeed
        let ptr1 = allocator.allocate(layout).unwrap();
        let ptr2 = allocator.allocate(layout).unwrap();

        // They may have the same address
        let _addr1 = ptr1.cast::<u8>().as_ptr() as usize;
        let _addr2 = ptr2.cast::<u8>().as_ptr() as usize;

        allocator.deallocate(ptr1.cast(), layout);
        allocator.deallocate(ptr2.cast(), layout);
    }
}

/// Test large allocations under Miri
#[test]
fn miri_large_allocation() {
    let config = BumpConfig {
        alloc_pattern: None,
        ..Default::default()
    };
    let allocator = BumpAllocator::with_config(10 * 1024 * 1024, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();

        let ptr = allocator.allocate(layout).unwrap();

        // Write to first and last bytes
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0xAA, 1);
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr().add(1024 * 1024 - 1), 0xBB, 1);

        // Verify
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0xAA);
        assert_eq!(*ptr.cast::<u8>().as_ptr().add(1024 * 1024 - 1), 0xBB);

        allocator.deallocate(ptr.cast(), layout);
    }
}

/// Test concurrent access patterns under Miri (single-threaded)
#[test]
fn miri_sequential_access() {
    let allocator = PoolAllocator::with_config(128, 8, 64, PoolConfig::default()).unwrap();

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();

        for iteration in 0..10 {
            let mut ptrs = Vec::new();

            // Allocate batch
            for i in 0..8 {
                let ptr = allocator.allocate(layout).unwrap();
                std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), (iteration * 10 + i) as u8, 128);
                ptrs.push(ptr);
            }

            // Verify
            for (i, ptr) in ptrs.iter().enumerate() {
                assert_eq!(*ptr.cast::<u8>().as_ptr(), (iteration * 10 + i) as u8);
            }

            // Deallocate
            for ptr in ptrs {
                allocator.deallocate(ptr.cast(), layout);
            }
        }
    }
}
