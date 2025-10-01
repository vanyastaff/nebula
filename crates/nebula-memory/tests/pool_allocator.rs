//! Integration tests for Pool allocator

use nebula_memory::allocator::{Allocator, PoolAllocator, PoolConfig};
use std::alloc::Layout;

#[test]
fn test_pool_allocator_basic() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(128, 8, 16, config)
        .expect("Failed to create pool allocator");

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();
        let ptr = allocator.allocate(layout).expect("Allocation failed");

        // Write to allocated memory
        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), 0x42, 128);
        assert_eq!(*ptr.cast::<u8>().as_ptr(), 0x42);

        // Deallocate
        allocator.deallocate(ptr.cast(), layout);
    }
}

#[test]
fn test_pool_allocator_reuse() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(64, 8, 16, config)
        .expect("Failed to create pool allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Allocate
        let ptr1 = allocator.allocate(layout).expect("First allocation failed");
        let addr1 = ptr1.cast::<u8>().as_ptr() as usize;

        // Deallocate
        allocator.deallocate(ptr1.cast(), layout);

        // Allocate again - should reuse the same block
        let ptr2 = allocator.allocate(layout).expect("Second allocation failed");
        let addr2 = ptr2.cast::<u8>().as_ptr() as usize;

        // Pool allocators typically reuse freed blocks
        assert_eq!(addr1, addr2, "Pool should reuse freed blocks");

        allocator.deallocate(ptr2.cast(), layout);
    }
}

#[test]
fn test_pool_allocator_multiple_blocks() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(32, 8, 16, config)
        .expect("Failed to create pool allocator");

    unsafe {
        let layout = Layout::from_size_align(32, 8).unwrap();

        // Allocate multiple blocks
        let mut ptrs = vec![];
        for i in 0..10 {
            let ptr = allocator.allocate(layout).expect("Allocation failed");
            std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), i as u8, 32);
            ptrs.push(ptr);
        }

        // Verify all blocks are different
        for i in 0..ptrs.len() {
            for j in (i + 1)..ptrs.len() {
                assert_ne!(ptrs[i].as_ptr(), ptrs[j].as_ptr());
            }
        }

        // Verify patterns
        for (i, ptr) in ptrs.iter().enumerate() {
            assert_eq!(*ptr.cast::<u8>().as_ptr(), i as u8);
        }

        // Deallocate all
        for ptr in ptrs {
            allocator.deallocate(ptr.cast(), layout);
        }
    }
}

#[test]
fn test_pool_allocator_alignment() {
    let config = PoolConfig::default();

    unsafe {
        // Test 8-byte alignment
        let alloc8 = PoolAllocator::with_config(64, 8, 16, config).unwrap();
        let layout8 = Layout::from_size_align(64, 8).unwrap();
        let ptr8 = alloc8.allocate(layout8).unwrap();
        assert_eq!(ptr8.cast::<u8>().as_ptr() as usize % 8, 0);
        alloc8.deallocate(ptr8.cast(), layout8);

        // Test 16-byte alignment
        let alloc16 = PoolAllocator::with_config(64, 16, 16, config).unwrap();
        let layout16 = Layout::from_size_align(64, 16).unwrap();
        let ptr16 = alloc16.allocate(layout16).unwrap();
        assert_eq!(ptr16.cast::<u8>().as_ptr() as usize % 16, 0);
        alloc16.deallocate(ptr16.cast(), layout16);

        // Test 32-byte alignment
        let alloc32 = PoolAllocator::with_config(64, 32, 16, config).unwrap();
        let layout32 = Layout::from_size_align(64, 32).unwrap();
        let ptr32 = alloc32.allocate(layout32).unwrap();
        assert_eq!(ptr32.cast::<u8>().as_ptr() as usize % 32, 0);
        alloc32.deallocate(ptr32.cast(), layout32);
    }
}

#[test]
fn test_pool_allocator_concurrent() {
    use std::sync::Arc;
    use std::thread;

    let config = PoolConfig::default();
    let allocator = Arc::new(
        PoolAllocator::with_config(128, 8, 16, config).expect("Failed to create allocator")
    );

    let mut handles = vec![];

    // Spawn multiple threads
    for i in 0..4 {
        let alloc = Arc::clone(&allocator);
        let handle = thread::spawn(move || {
            unsafe {
                let layout = Layout::from_size_align(128, 8).unwrap();
                let mut ptrs = vec![];

                // Each thread allocates 5 blocks
                for _ in 0..5 {
                    if let Ok(ptr) = alloc.allocate(layout) {
                        std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), i as u8, 128);
                        ptrs.push(ptr);
                    }
                }

                // Verify patterns
                for ptr in &ptrs {
                    assert_eq!(*ptr.cast::<u8>().as_ptr(), i as u8);
                }

                // Deallocate
                for ptr in ptrs {
                    alloc.deallocate(ptr.cast(), layout);
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

#[test]
fn test_pool_allocator_stress() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(256, 8, 16, config)
        .expect("Failed to create pool allocator");

    unsafe {
        let layout = Layout::from_size_align(256, 8).unwrap();

        // Allocate and deallocate many times
        for iteration in 0..100 {
            let mut ptrs = vec![];

            // Allocate 10 blocks
            for _ in 0..10 {
                if let Ok(ptr) = allocator.allocate(layout) {
                    std::ptr::write_bytes(ptr.cast::<u8>().as_ptr(), iteration as u8, 256);
                    ptrs.push(ptr);
                }
            }

            // Verify
            for ptr in &ptrs {
                assert_eq!(*ptr.cast::<u8>().as_ptr(), iteration as u8);
            }

            // Deallocate
            for ptr in ptrs {
                allocator.deallocate(ptr.cast(), layout);
            }
        }
    }
}

#[test]
fn test_pool_allocator_partial_deallocation() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(64, 8, 16, config)
        .expect("Failed to create pool allocator");

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Allocate 5 blocks
        let mut ptrs = vec![];
        for _ in 0..5 {
            ptrs.push(allocator.allocate(layout).expect("Allocation failed"));
        }

        // Deallocate only 2nd and 4th blocks
        allocator.deallocate(ptrs[1], layout);
        allocator.deallocate(ptrs[3], layout);

        // Allocate 2 more - should reuse freed slots
        let ptr_new1 = allocator.allocate(layout).expect("Reallocation 1 failed");
        let ptr_new2 = allocator.allocate(layout).expect("Reallocation 2 failed");

        // Cleanup
        allocator.deallocate(ptrs[0], layout);
        allocator.deallocate(ptr_new1.cast(), layout);
        allocator.deallocate(ptrs[2], layout);
        allocator.deallocate(ptr_new2.cast(), layout);
        allocator.deallocate(ptrs[4], layout);
    }
}
