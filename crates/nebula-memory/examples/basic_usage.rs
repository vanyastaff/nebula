//! Basic usage examples of nebula-memory allocators
//!
//! This example demonstrates the fundamental usage patterns for different allocators.

use nebula_memory::allocator::{BumpAllocator, PoolAllocator, StackAllocator};
use nebula_memory::prelude::*;
use std::alloc::Layout;

fn main() {
    println!("=== nebula-memory Basic Usage Examples ===\n");

    // Example 1: Bump Allocator
    bump_allocator_example();

    // Example 2: Pool Allocator
    pool_allocator_example();

    // Example 3: Stack Allocator
    stack_allocator_example();
}

fn bump_allocator_example() {
    println!("## Bump Allocator Example");
    println!("Use case: Fast sequential allocations, bulk deallocation\n");

    // Create a 1MB bump allocator
    let allocator = BumpAllocator::production(1024 * 1024).expect("Failed to create allocator");

    // Allocate some memory
    let layout = Layout::from_size_align(64, 8).unwrap();
    unsafe {
        let ptr1 = allocator.allocate(layout).expect("Allocation failed");
        println!("  Allocated 64 bytes at {:?}", ptr1.as_ptr());

        let ptr2 = allocator.allocate(layout).expect("Allocation failed");
        println!("  Allocated 64 bytes at {:?}", ptr2.as_ptr());
    }

    // Use checkpoint/restore for scope-based memory management
    let checkpoint = allocator.checkpoint();
    println!("  Created checkpoint at position: {:?}", allocator.used());

    unsafe {
        let _temp = allocator.allocate(layout).expect("Allocation failed");
        println!("  Allocated temporary 64 bytes");
        println!("  Used memory: {}", allocator.used());
    }

    // Restore to checkpoint (like a scope cleanup)
    allocator.restore(checkpoint).expect("Restore failed");
    println!("  Restored to checkpoint");
    println!("  Used memory after restore: {}\n", allocator.used());
}

fn pool_allocator_example() {
    println!("## Pool Allocator Example");
    println!("Use case: Fixed-size objects, object reuse\n");

    // Create a pool for 64-byte objects
    let allocator = PoolAllocator::production(64, 8, 100).expect("Failed to create pool");

    println!("  Created pool for 100 objects of 64 bytes each");
    println!("  Available blocks: {}", allocator.available());

    // Allocate some blocks
    let layout = Layout::from_size_align(64, 8).unwrap();
    unsafe {
        let ptr1 = allocator.allocate(layout).expect("Allocation failed");
        let ptr2 = allocator.allocate(layout).expect("Allocation failed");

        println!("  Allocated 2 blocks");
        println!("  Available blocks: {}", allocator.available());

        // Deallocate returns blocks to the pool for reuse
        allocator.deallocate(ptr1.as_non_null_ptr(), layout);
        allocator.deallocate(ptr2.as_non_null_ptr(), layout);

        println!("  Deallocated 2 blocks");
        println!("  Available blocks: {}\n", allocator.available());
    }
}

fn stack_allocator_example() {
    println!("## Stack Allocator Example");
    println!("Use case: LIFO allocations, temporary scratch space\n");

    // Create a 64KB stack allocator
    let allocator = StackAllocator::production(64 * 1024).expect("Failed to create allocator");

    // Push a frame
    let frame = allocator.push_frame();
    println!("  Pushed frame, depth: {}", allocator.depth());

    let layout = Layout::from_size_align(128, 8).unwrap();
    unsafe {
        let _ptr = allocator.allocate(layout).expect("Allocation failed");
        println!("  Allocated 128 bytes in frame");
        println!("  Used memory: {}", allocator.used());
    }

    // Pop frame deallocates everything in that frame
    drop(frame);
    println!("  Popped frame");
    println!("  Used memory after pop: {}", allocator.used());
    println!();
}
