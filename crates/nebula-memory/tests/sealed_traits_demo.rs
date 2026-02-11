//! Demonstration of sealed traits pattern in nebula-memory
//!
//! This test shows how sealed traits prevent external implementations
//! while allowing use as trait bounds.

use core::alloc::Layout;
use nebula_memory::allocator::bump::{BumpAllocator, BumpConfig};
use nebula_memory::allocator::sealed::AllocatorInternal;
use nebula_memory::allocator::{Allocator, Resettable};

#[test]
fn test_sealed_trait_usage() {
    // ✅ Can create allocators that implement sealed trait
    let mut alloc = BumpAllocator::with_config(1024, BumpConfig::default()).unwrap();

    // ✅ Can use sealed trait methods
    let checkpoint = alloc.internal_checkpoint();
    println!("Checkpoint: {:?}", checkpoint);

    // ✅ Can use sealed trait as bound in generic functions
    analyze_allocator(&alloc);

    // Make an allocation
    unsafe {
        let layout = Layout::new::<u64>();
        let _ptr = alloc.allocate(layout).unwrap();
    }

    // ✅ Can restore checkpoint
    unsafe {
        alloc.internal_restore(checkpoint).unwrap();
    }

    // ✅ Can get fragmentation stats
    let stats = alloc.internal_fragmentation();
    assert_eq!(stats.fragmentation_percent, 0); // Bump allocators have 0% fragmentation

    // ✅ Can validate (debug builds)
    #[cfg(debug_assertions)]
    {
        alloc.internal_validate().unwrap();
    }

    // ✅ Can get type name
    assert_eq!(alloc.internal_type_name(), "BumpAllocator");
}

#[test]
fn test_checkpoint_validation() {
    let mut alloc = BumpAllocator::with_config(1024, BumpConfig::default()).unwrap();

    // Create checkpoint
    let checkpoint = alloc.internal_checkpoint();

    // Make allocations
    unsafe {
        let layout = Layout::from_size_align(100, 8).unwrap();
        let _ptr = alloc.allocate(layout).unwrap();
    }

    // Restore should work
    unsafe {
        let result = alloc.internal_restore(checkpoint);
        assert!(result.is_ok());
    }
}

#[test]
fn test_stale_checkpoint_detection() {
    let mut alloc = BumpAllocator::with_config(1024, BumpConfig::default()).unwrap();

    // Create checkpoint
    let checkpoint = alloc.internal_checkpoint();

    // Reset allocator (increments generation)
    unsafe {
        alloc.reset();
    }

    // Checkpoint should now be stale
    unsafe {
        let result = alloc.internal_restore(checkpoint);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("stale"));
        }
    }
}

#[test]
fn test_fragmentation_stats() {
    let alloc = BumpAllocator::with_config(1024, BumpConfig::default()).unwrap();

    let stats = alloc.internal_fragmentation();

    // Bump allocators have perfect contiguity
    assert_eq!(stats.fragment_count, 1); // Single contiguous block
    assert_eq!(stats.total_free, stats.largest_block); // All free space is one block
    assert_eq!(stats.fragmentation_percent, 0); // 0% fragmentation
    assert!(!stats.is_fragmented()); // Not fragmented
}

// ============================================================================
// Generic function using sealed trait as bound
// ============================================================================

/// Example function that uses AllocatorInternal as a trait bound
///
/// ✅ This compiles - external users can use sealed traits as bounds
fn analyze_allocator<A: AllocatorInternal>(alloc: &A) {
    let checkpoint = alloc.internal_checkpoint();
    let stats = alloc.internal_fragmentation();
    let type_name = alloc.internal_type_name();

    println!("Analyzing allocator: {}", type_name);
    println!("  Checkpoint: {:?}", checkpoint);
    println!("  Fragmentation: {}%", stats.fragmentation_percent);
    println!("  Free space: {} bytes", stats.total_free);
}

// ============================================================================
// External implementation attempt (would fail to compile)
// ============================================================================

// Uncomment this to see the sealed trait in action:
//
// struct MyAllocator;
//
// // ❌ This would fail to compile:
// // error: private trait `Sealed` cannot be implemented outside its module
// impl AllocatorInternal for MyAllocator {
//     fn internal_checkpoint(&self) -> InternalCheckpoint {
//         todo!()
//     }
//
//     unsafe fn internal_restore(&mut self, checkpoint: InternalCheckpoint) {
//         todo!()
//     }
// }
