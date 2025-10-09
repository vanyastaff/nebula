//! Comprehensive error handling patterns for nebula-memory
//!
//! This example demonstrates best practices for handling allocation errors,
//! graceful degradation, and error recovery strategies.

use nebula_memory::allocator::{AllocError, BumpAllocator, PoolAllocator, TypedAllocator};
use nebula_memory::prelude::*;
use std::ptr::NonNull;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Error Handling Examples ===\n");

    // Example 1: Graceful Degradation
    println!("1. Graceful Degradation:");
    graceful_degradation()?;
    println!();

    // Example 2: Error Recovery
    println!("2. Error Recovery:");
    error_recovery()?;
    println!();

    // Example 3: Fallback Allocator Chain
    println!("3. Fallback Allocator Chain:");
    fallback_chain()?;
    println!();

    // Example 4: Rich Error Messages
    println!("4. Rich Error Messages:");
    rich_error_messages()?;
    println!();

    Ok(())
}

/// Demonstrate graceful degradation when allocator fails
fn graceful_degradation() -> Result<(), Box<dyn std::error::Error>> {
    let allocator = BumpAllocator::new(1024)?;

    // Try to allocate more than available
    match unsafe { allocator.alloc::<[u8; 2048]>() } {
        Ok(ptr) => {
            println!("✓ Custom allocator succeeded");
            unsafe { allocator.dealloc(ptr) };
        }
        Err(e) => {
            eprintln!("⚠ Custom allocator failed: {}", e);
            println!("↪ Falling back to system allocator");

            // Fallback to Box (system allocator)
            let data = Box::new([0u8; 2048]);
            println!("✓ System allocator succeeded: {} bytes", data.len());
        }
    }

    Ok(())
}

/// Demonstrate error recovery by freeing memory
fn error_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let allocator = PoolAllocator::new(64, 8, 10)?;
    let mut ptrs = Vec::new();

    // Fill the pool
    println!("Filling pool (10 blocks)...");
    for i in 0..10 {
        match unsafe { allocator.alloc::<[u8; 64]>() } {
            Ok(ptr) => {
                ptrs.push(ptr);
                print!(".");
            }
            Err(e) => {
                println!("\n✗ Allocation {} failed: {}", i + 1, e);
                break;
            }
        }
    }
    println!(" Done");

    // Try one more (will fail)
    println!("\nAttempting allocation when pool is full...");
    match unsafe { allocator.alloc::<[u8; 64]>() } {
        Ok(_) => println!("✓ Unexpected success"),
        Err(e) => {
            println!("✗ Expected failure:");
            println!("{}", e);
            println!("\n↪ Recovering: freeing 5 blocks...");

            // Free half the allocations
            for ptr in ptrs.drain(..5) {
                unsafe { allocator.dealloc(ptr) };
            }

            // Try again
            match unsafe { allocator.alloc::<[u8; 64]>() } {
                Ok(_) => println!("✓ Recovery successful!"),
                Err(e) => println!("✗ Recovery failed: {}", e),
            }
        }
    }

    // Cleanup
    for ptr in ptrs {
        unsafe { allocator.dealloc(ptr) };
    }

    Ok(())
}

/// Demonstrate fallback allocator chain
fn fallback_chain() -> Result<(), Box<dyn std::error::Error>> {
    struct AllocatorChain {
        primary: BumpAllocator,
        secondary: PoolAllocator,
    }

    impl AllocatorChain {
        fn allocate_with_fallback<T>(&self) -> Result<NonNull<T>, AllocError> {
            unsafe {
                // Try primary first
                self.primary.alloc::<T>().or_else(|e1| {
                    eprintln!("  Primary allocator failed: {}", e1.inner().error_code());
                    // Fall back to secondary
                    self.secondary.alloc::<T>().map_err(|e2| {
                        eprintln!("  Secondary allocator failed: {}", e2.inner().error_code());
                        e2
                    })
                })
            }
        }
    }

    let chain = AllocatorChain {
        primary: BumpAllocator::new(64)?,
        secondary: PoolAllocator::new(64, 8, 100)?,
    };

    // This will use primary (fits in 64 bytes)
    println!("Allocating u64 (8 bytes)...");
    let ptr1 = chain.allocate_with_fallback::<u64>()?;
    println!("✓ Allocated from primary allocator");

    // This will overflow to secondary (doesn't fit in remaining space)
    println!("\nAllocating [u8; 128] (128 bytes)...");
    let ptr2 = chain.allocate_with_fallback::<[u8; 128]>()?;
    println!("✓ Allocated from secondary allocator (fallback)");

    // Cleanup
    unsafe {
        chain.primary.dealloc(ptr1);
        chain.secondary.dealloc(ptr2);
    }

    Ok(())
}

/// Demonstrate rich error message display
fn rich_error_messages() -> Result<(), Box<dyn std::error::Error>> {
    println!("Attempting to allocate beyond capacity...");

    let allocator = BumpAllocator::new(256)?;

    // Allocate some memory first
    let _ptr1 = unsafe { allocator.alloc::<[u8; 100]>()? };
    println!("✓ Allocated 100 bytes");

    let _ptr2 = unsafe { allocator.alloc::<[u8; 100]>()? };
    println!("✓ Allocated 100 bytes");

    // This will fail with rich error message
    println!("\nAttempting to allocate 200 bytes (insufficient space):");
    match unsafe { allocator.alloc::<[u8; 200]>() } {
        Ok(_) => println!("Unexpected success"),
        Err(e) => {
            println!("\n{}", e);

            // Error provides useful information
            if let Some(layout) = e.layout() {
                println!("Requested layout: {} bytes, align {}",
                    layout.size(), layout.align());
            }

            if let Some(suggestion) = e.suggestion() {
                println!("Suggestion available: {}", !suggestion.is_empty());
            }
        }
    }

    Ok(())
}
