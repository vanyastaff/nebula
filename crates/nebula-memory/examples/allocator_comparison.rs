//! Allocator comparison example
//!
//! Demonstrates when to use each allocator type

use nebula_memory::allocator::{Allocator, BumpAllocator, PoolAllocator, PoolConfig, StackAllocator, StackConfig, StackFrame};
use nebula_memory::core::traits::Resettable;
use std::alloc::Layout;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Allocator Comparison Demo ===\n");

    demo_bump_allocator()?;
    demo_pool_allocator()?;
    demo_stack_allocator()?;

    Ok(())
}

/// BumpAllocator: Best for request-scoped allocations
fn demo_bump_allocator() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- BumpAllocator ---");
    println!("Use case: HTTP request handling, temporary computations\n");

    let allocator = BumpAllocator::new(1024)?;

    // Simulate processing a request
    for request_id in 1..=3 {
        println!("Processing request {}", request_id);

        unsafe {
            // Allocate request data
            let layout = Layout::from_size_align(64, 8)?;
            let request_data = allocator.allocate(layout)?;

            // Use the data
            std::ptr::write_bytes(request_data.cast::<u8>().as_ptr(), 0x42, 64);

            println!("  Allocated and processed {} bytes", 64);
        }

        // Reset after each request (fast!)
        unsafe { allocator.reset(); }
        println!("  Reset allocator for next request\n");
    }

    println!("BumpAllocator advantages:");
    println!("  ✓ Extremely fast allocation");
    println!("  ✓ Zero fragmentation");
    println!("  ✓ Bulk deallocation (reset)");
    println!("  ✗ No individual deallocation");
    println!("  ✗ Higher memory usage\n");

    Ok(())
}

/// PoolAllocator: Best for fixed-size object pools
fn demo_pool_allocator() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- PoolAllocator ---");
    println!("Use case: Database connections, worker threads, reusable objects\n");

    let config = PoolConfig::default();
    let allocator = PoolAllocator::with_config(128, 8, 10, config)?;

    unsafe {
        let layout = Layout::from_size_align(128, 8)?;
        let mut objects = Vec::new();

        // Allocate objects
        for i in 1..=5 {
            let obj = allocator.allocate(layout)?;
            println!("Allocated object {}", i);
            objects.push(obj);
        }

        // Return objects to pool
        for (i, obj) in objects.iter().enumerate() {
            allocator.deallocate(obj.cast(), layout);
            println!("Returned object {} to pool", i + 1);
        }

        println!("\nReallocating (will reuse freed blocks):");

        // Reallocate - should reuse pool blocks
        for i in 1..=5 {
            let obj = allocator.allocate(layout)?;
            println!("Reallocated object {} (reused from pool)", i);
            allocator.deallocate(obj.cast(), layout);
        }
    }

    println!("\nPoolAllocator advantages:");
    println!("  ✓ Excellent memory reuse");
    println!("  ✓ Predictable performance");
    println!("  ✓ Low fragmentation");
    println!("  ✗ Fixed block size");
    println!("  ✗ Memory overhead per block\n");

    Ok(())
}

/// StackAllocator: Best for LIFO patterns
fn demo_stack_allocator() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- StackAllocator ---");
    println!("Use case: Nested scopes, recursive algorithms\n");

    let config = StackConfig::default();
    let allocator = StackAllocator::with_config(2048, config)?;

    unsafe {
        // Outer scope
        println!("Entering outer scope");
        let _frame1 = StackFrame::new(&allocator);

        let layout = Layout::from_size_align(64, 8)?;
        let outer_data = allocator.allocate(layout)?;
        println!("  Allocated outer data");

        {
            // Inner scope
            println!("  Entering inner scope");
            let _frame2 = StackFrame::new(&allocator);

            let inner_data = allocator.allocate(layout)?;
            println!("    Allocated inner data");

            // Inner frame automatically deallocates on drop
            println!("  Exiting inner scope (auto-cleanup)");
        }

        println!("Back to outer scope");
        // Outer frame still valid, can allocate more
        let more_data = allocator.allocate(layout)?;
        println!("  Allocated more data in outer scope");

        println!("Exiting outer scope (auto-cleanup)");
    }

    println!("\nStackAllocator advantages:");
    println!("  ✓ RAII-style automatic cleanup");
    println!("  ✓ Perfect for nested scopes");
    println!("  ✓ Marker-based bulk deallocation");
    println!("  ✗ Requires LIFO discipline");
    println!("  ✗ Cannot deallocate out of order\n");

    Ok(())
}
