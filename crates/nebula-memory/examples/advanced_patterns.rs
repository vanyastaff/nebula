//! Advanced memory management patterns
//!
//! Shows sophisticated usage of allocators

use nebula_memory::allocator::{Allocator, BumpAllocator, PoolAllocator, PoolConfig};
use nebula_memory::core::traits::Resettable;
use std::alloc::Layout;
use std::sync::Arc;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Advanced Memory Management Patterns ===\n");

    arena_pattern()?;
    object_pool_pattern()?;
    thread_local_pattern()?;

    Ok(())
}

/// Arena pattern: Allocate many, deallocate all at once
fn arena_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Arena Pattern ---");
    println!("Allocate many objects, bulk deallocate\n");

    let allocator = BumpAllocator::new(4096)?;
    let layout = Layout::from_size_align(32, 8)?;

    unsafe {
        let mut nodes = Vec::new();

        // Build a tree/graph structure
        for i in 0..50 {
            let node = allocator.allocate(layout)?;
            std::ptr::write_bytes(node.cast::<u8>().as_ptr(), i as u8, 32);
            nodes.push(node);
        }

        println!("Allocated {} nodes", nodes.len());

        // Process the structure
        for (i, node) in nodes.iter().enumerate() {
            let value = *node.cast::<u8>().as_ptr();
            if i < 3 || i >= nodes.len() - 3 {
                println!("  Node {}: value = {}", i, value);
            } else if i == 3 {
                println!("  ...");
            }
        }

        // Bulk deallocate - O(1) operation!
        let start = std::time::Instant::now();
        allocator.reset();
        let duration = start.elapsed();

        println!("\nReset {} nodes in {:?}", nodes.len(), duration);
        println!("Arena pattern is ideal for:");
        println!("  • AST/IR construction");
        println!("  • Graph algorithms");
        println!("  • Temporary data structures\n");
    }

    Ok(())
}

/// Object pool pattern: Reuse expensive objects
fn object_pool_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Object Pool Pattern ---");
    println!("Reuse expensive objects (e.g., database connections)\n");

    let config = PoolConfig::default();
    let pool = PoolAllocator::with_config(256, 8, 20, config)?;

    unsafe {
        let layout = Layout::from_size_align(256, 8)?;

        // Simulate connection pool
        println!("Creating connection pool...");

        let mut active_connections = Vec::new();

        // Acquire connections
        for i in 1..=5 {
            let conn = pool.allocate(layout)?;
            // Initialize connection
            std::ptr::write_bytes(conn.cast::<u8>().as_ptr(), 0xFF, 256);
            println!("  Acquired connection {}", i);
            active_connections.push(conn);
        }

        // Release some connections
        for i in 0..3 {
            pool.deallocate(active_connections[i].cast(), layout);
            println!("  Released connection {}", i + 1);
        }
        active_connections.drain(0..3);

        // Acquire more (will reuse released connections)
        println!("\nAcquiring more connections (reusing from pool):");
        for i in 1..=3 {
            let conn = pool.allocate(layout)?;
            println!("  Acquired connection {} (reused)", i);
            active_connections.push(conn);
        }

        // Cleanup
        for (i, conn) in active_connections.iter().enumerate() {
            pool.deallocate(conn.cast(), layout);
            println!("  Closed connection {}", i + 1);
        }

        println!("\nObject pool pattern benefits:");
        println!("  • Reduces allocation overhead");
        println!("  • Reuses initialized objects");
        println!("  • Predictable performance\n");
    }

    Ok(())
}

/// Thread-local pattern: Per-thread allocators
fn thread_local_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Thread-Local Allocators ---");
    println!("Each thread has its own allocator\n");

    let allocator = Arc::new(PoolAllocator::with_config(
        128,
        8,
        100,
        PoolConfig::default(),
    )?);
    let mut handles = Vec::new();

    // Spawn worker threads
    for thread_id in 0..4 {
        let allocator_clone = Arc::clone(&allocator);

        let handle = thread::spawn(move || {
            unsafe {
                let layout = Layout::from_size_align(128, 8).unwrap();
                let mut local_data = Vec::new();

                // Each thread allocates its own data
                for i in 0..10 {
                    let data = allocator_clone.allocate(layout).unwrap();
                    std::ptr::write_bytes(data.cast::<u8>().as_ptr(), thread_id as u8, 128);
                    local_data.push(data);
                }

                println!("Thread {} allocated {} blocks", thread_id, local_data.len());

                // Thread-local processing
                thread::sleep(std::time::Duration::from_millis(10));

                // Cleanup
                for data in local_data {
                    allocator_clone.deallocate(data.cast(), layout);
                }

                println!("Thread {} deallocated all blocks", thread_id);
            }
        });

        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    println!("\nThread-local pattern advantages:");
    println!("  • Reduces contention");
    println!("  • Better cache locality");
    println!("  • Scalable performance\n");

    Ok(())
}
