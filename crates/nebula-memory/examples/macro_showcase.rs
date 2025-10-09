//! Showcase of nebula-memory ergonomic macros
//!
//! This example demonstrates the powerful macro DSL for simplified
//! memory management with allocators.

use nebula_memory::{allocator, alloc, dealloc, memory_scope, budget};
use nebula_memory::allocator::{BumpAllocator, TypedAllocator};
use nebula_memory::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Macro Showcase ===\n");

    // Example 1: Ergonomic Allocator Creation
    println!("1. Ergonomic Allocator Creation:");
    allocator_creation()?;
    println!();

    // Example 2: Type-safe Allocation/Deallocation
    println!("2. Type-safe Allocation:");
    type_safe_allocation()?;
    println!();

    // Example 3: Memory Scopes
    println!("3. Memory Scopes:");
    memory_scopes()?;
    println!();

    // Example 4: Memory Budgets
    println!("4. Memory Budgets:");
    memory_budgets()?;
    println!();

    Ok(())
}

/// Demonstrate ergonomic allocator creation with allocator! macro
fn allocator_creation() -> Result<(), Box<dyn std::error::Error>> {
    // Simple creation
    let bump = allocator!(bump 4096)?;
    println!("✓ Created bump allocator: {} bytes", bump.capacity());

    let pool = allocator!(pool 64, 100)?;
    println!("✓ Created pool allocator: {} blocks", pool.block_count());

    let stack = allocator!(stack 8192)?;
    println!("✓ Created stack allocator: {} bytes", stack.capacity());

    // With configuration
    let bump_debug = allocator!(bump 4096, {
        thread_safe: true,
        track_stats: true,
    })?;
    println!("✓ Created configured bump allocator with stats tracking");

    Ok(())
}

/// Demonstrate type-safe allocation with alloc!/dealloc! macros
fn type_safe_allocation() -> Result<(), Box<dyn std::error::Error>> {
    let allocator = allocator!(bump 4096)?;

    // Allocate single value
    println!("Allocating single u64...");
    let ptr = unsafe { alloc!(allocator, u64) }?;
    unsafe { ptr.as_ptr().write(42) };
    println!("✓ Allocated and initialized: {}", unsafe { *ptr.as_ptr() });

    // Allocate with initialization
    println!("\nAllocating with initialization...");
    let ptr2 = unsafe { alloc!(allocator, u64 = 100) }?;
    println!("✓ Allocated with value: {}", unsafe { *ptr2.as_ptr() });

    // Allocate array
    println!("\nAllocating array...");
    let arr = unsafe { alloc!(allocator, [u32; 5]) }?;
    unsafe {
        for i in 0..5 {
            arr.as_ptr().add(i).write(i as u32 * 10);
        }
    }
    print!("✓ Allocated array: [");
    unsafe {
        for i in 0..5 {
            print!("{}{}", *arr.as_ptr().add(i), if i < 4 { ", " } else { "" });
        }
    }
    println!("]");

    // Type-safe deallocation
    unsafe {
        dealloc!(allocator, ptr, u64);
        dealloc!(allocator, ptr2, u64);
        dealloc!(allocator, arr, [u32; 5]);
    }
    println!("\n✓ All memory deallocated type-safely");

    Ok(())
}

/// Demonstrate memory scopes with automatic cleanup
fn memory_scopes() -> Result<(), Box<dyn std::error::Error>> {
    let allocator = BumpAllocator::new(4096)?;

    println!("Initial usage: {} bytes", allocator.used());

    // Scope 1: Allocate and auto-free
    let result = memory_scope!(allocator, {
        println!("  Scope 1: Allocating 100 bytes...");
        let ptr = unsafe { allocator.alloc::<[u8; 100]>()? };
        println!("  Usage in scope: {} bytes", allocator.used());
        unsafe { ptr.as_ptr().write([42u8; 100]) };
        Ok(unsafe { (*ptr.as_ptr())[0] })
    })?;

    println!("✓ Scope 1 exited: result = {}", result);
    println!("✓ Usage after scope: {} bytes (memory freed!)", allocator.used());

    // Scope 2: Nested scopes
    memory_scope!(allocator, {
        println!("\n  Scope 2 (outer): Allocating 200 bytes...");
        let _ptr1 = unsafe { allocator.alloc::<[u8; 200]>()? };
        println!("  Usage: {} bytes", allocator.used());

        memory_scope!(allocator, {
            println!("    Scope 2 (inner): Allocating 300 bytes...");
            let _ptr2 = unsafe { allocator.alloc::<[u8; 300]>()? };
            println!("    Usage: {} bytes", allocator.used());
            Ok(())
        })?;

        println!("  Inner scope freed, usage: {} bytes", allocator.used());
        Ok(())
    })?;

    println!("✓ All scopes exited, usage: {} bytes", allocator.used());

    Ok(())
}

/// Demonstrate memory budget creation
fn memory_budgets() -> Result<(), Box<dyn std::error::Error>> {
    // Simple budget
    let budget1 = budget!(10 * 1024 * 1024); // 10MB
    println!("✓ Created simple budget: {} bytes total", budget1.total_limit());

    // Budget with per-allocation limit
    let budget2 = budget!(
        total: 100 * 1024 * 1024,
        per_alloc: 1024 * 1024
    );
    println!("✓ Created budget: {} MB total, {} MB per allocation",
        budget2.total_limit() / (1024 * 1024),
        budget2.allocation_limit() / (1024 * 1024));

    // Use budget to track allocations
    println!("\nTracking allocations against budget...");
    budget2.try_allocate(512 * 1024)?;
    println!("  ✓ Allocated 512 KB");

    budget2.try_allocate(256 * 1024)?;
    println!("  ✓ Allocated 256 KB");

    println!("  Budget used: {} KB / {} MB",
        budget2.used() / 1024,
        budget2.total_limit() / (1024 * 1024));

    Ok(())
}
