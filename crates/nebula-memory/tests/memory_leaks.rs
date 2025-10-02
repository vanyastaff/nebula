//! Memory leak detection tests
//!
//! These tests verify that allocators properly free memory and don't leak.

use nebula_memory::allocator::bump::BumpConfig;
use nebula_memory::allocator::{
    Allocator, BumpAllocator, PoolAllocator, PoolConfig, StackAllocator, StackConfig,
};
use nebula_memory::core::traits::{MemoryUsage, Resettable};
use std::alloc::Layout;

/// Test that BumpAllocator resets properly
#[test]
fn test_bump_allocator_reset_clears_memory() {
    let config = BumpConfig::default();
    let allocator = BumpAllocator::with_config(4096, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(256, 8).unwrap();

        // Allocate some memory
        for _ in 0..10 {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
        }

        // Check usage before reset
        let usage_before = allocator.used_memory();
        assert!(usage_before > 0, "Memory should be used before reset");

        // Reset should clear everything
        allocator.reset();

        // Check usage after reset
        let usage_after = allocator.used_memory();
        assert_eq!(usage_after, 0, "Memory usage should be 0 after reset");
    }
}

/// Test that PoolAllocator properly tracks allocations
#[test]
fn test_pool_allocator_tracks_usage() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(128, 8, 16, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(128, 8).unwrap();
        let mut ptrs = Vec::new();

        // Allocate blocks
        for _ in 0..8 {
            let ptr = allocator.allocate(layout).unwrap();
            ptrs.push(ptr);
        }

        let usage_allocated = allocator.used_memory();
        assert_eq!(usage_allocated, 8 * 128, "Should track allocated memory");

        // Deallocate all
        for ptr in ptrs {
            allocator.deallocate(ptr.cast(), layout);
        }

        // Pool still owns the memory, but blocks are free
        let usage_freed = allocator.used_memory();
        assert_eq!(usage_freed, 0, "Usage should be 0 after deallocation");
    }
}

/// Test that StackAllocator properly frees in LIFO order
#[test]
fn test_stack_allocator_lifo_deallocation() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(8192, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(256, 8).unwrap();

        // Allocate in order
        let ptr1 = allocator.allocate(layout).unwrap();
        let usage1 = allocator.used_memory();

        let ptr2 = allocator.allocate(layout).unwrap();
        let usage2 = allocator.used_memory();

        let ptr3 = allocator.allocate(layout).unwrap();
        let usage3 = allocator.used_memory();

        assert!(usage2 > usage1, "Usage should increase");
        assert!(usage3 > usage2, "Usage should increase");

        // Deallocate in LIFO order
        allocator.deallocate(ptr3.cast(), layout);
        let usage_after_3 = allocator.used_memory();
        assert_eq!(usage_after_3, usage2, "Usage should return to previous");

        allocator.deallocate(ptr2.cast(), layout);
        let usage_after_2 = allocator.used_memory();
        assert_eq!(usage_after_2, usage1, "Usage should return to previous");

        allocator.deallocate(ptr1.cast(), layout);
        let usage_after_1 = allocator.used_memory();
        assert_eq!(usage_after_1, 0, "Usage should be 0");
    }
}

/// Test repeated allocation/deallocation doesn't leak
#[test]
fn test_no_leaks_in_repeated_cycles() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(64, 8, 32, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();

        // Run many cycles
        for _ in 0..100 {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
        }

        // Usage should be 0 (all blocks returned to pool)
        let final_usage = allocator.used_memory();
        assert_eq!(final_usage, 0, "No memory should be leaked");
    }
}

/// Test that BumpAllocator releases memory on drop
#[test]
fn test_bump_allocator_releases_on_drop() {
    unsafe {
        let layout = Layout::from_size_align(256, 8).unwrap();

        {
            let config = BumpConfig::default();
            let allocator = BumpAllocator::with_config(16384, config).unwrap();

            // Allocate some memory (BumpAllocator doesn't reclaim on dealloc)
            for _ in 0..10 {
                let ptr = allocator.allocate(layout).unwrap();
                allocator.deallocate(ptr.cast(), layout);
            }

            assert!(allocator.used_memory() > 0, "Memory should be used");

            // allocator drops here - verifies Drop impl works
        }

        // If there's a leak, ASan or Valgrind would detect it
        // This test verifies Drop impl is called correctly
    }
}

/// Test PoolAllocator with many small allocations
#[test]
fn test_pool_allocator_stress() {
    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(32, 8, 256, config).unwrap();

    unsafe {
        let layout = Layout::from_size_align(32, 8).unwrap();

        // Allocate all blocks
        let mut ptrs = Vec::new();
        for _ in 0..256 {
            let ptr = allocator.allocate(layout).unwrap();
            ptrs.push(ptr);
        }

        let peak_usage = allocator.used_memory();
        assert_eq!(peak_usage, 256 * 32, "All blocks allocated");

        // Deallocate all
        for ptr in ptrs {
            allocator.deallocate(ptr.cast(), layout);
        }

        let final_usage = allocator.used_memory();
        assert_eq!(final_usage, 0, "All blocks should be freed");
    }
}

/// Test mixed allocation sizes with BumpAllocator
#[test]
fn test_bump_allocator_mixed_sizes() {
    let config = BumpConfig::default();
    let allocator = BumpAllocator::with_config(64 * 1024, config).unwrap();

    unsafe {
        let initial_usage = allocator.used_memory();
        assert_eq!(initial_usage, 0);

        // Allocate different sizes
        let small = allocator
            .allocate(Layout::from_size_align(16, 8).unwrap())
            .unwrap();
        let medium = allocator
            .allocate(Layout::from_size_align(256, 8).unwrap())
            .unwrap();
        let large = allocator
            .allocate(Layout::from_size_align(4096, 8).unwrap())
            .unwrap();

        let total_usage = allocator.used_memory();
        assert!(
            total_usage >= 16 + 256 + 4096,
            "Should track all allocations"
        );

        // Deallocate (BumpAllocator doesn't reuse)
        allocator.deallocate(small.cast(), Layout::from_size_align(16, 8).unwrap());
        allocator.deallocate(medium.cast(), Layout::from_size_align(256, 8).unwrap());
        allocator.deallocate(large.cast(), Layout::from_size_align(4096, 8).unwrap());

        // Usage should still be the same (bump doesn't reclaim)
        let usage_after_dealloc = allocator.used_memory();
        assert_eq!(usage_after_dealloc, total_usage, "Bump doesn't reclaim");

        // Reset clears everything
        allocator.reset();
        assert_eq!(allocator.used_memory(), 0);
    }
}

/// Test StackAllocator memory usage tracking
#[test]
fn test_stack_allocator_usage_tracking() {
    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(8192, config).unwrap();

    unsafe {
        assert_eq!(allocator.used_memory(), 0);

        let layout1 = Layout::from_size_align(128, 8).unwrap();
        let ptr1 = allocator.allocate(layout1).unwrap();
        let usage1 = allocator.used_memory();
        assert!(usage1 >= 128);

        let layout2 = Layout::from_size_align(256, 8).unwrap();
        let ptr2 = allocator.allocate(layout2).unwrap();
        let usage2 = allocator.used_memory();
        assert!(usage2 >= usage1 + 256);

        // Deallocate in LIFO order
        allocator.deallocate(ptr2.cast(), layout2);
        assert_eq!(allocator.used_memory(), usage1);

        allocator.deallocate(ptr1.cast(), layout1);
        assert_eq!(allocator.used_memory(), 0);
    }
}
