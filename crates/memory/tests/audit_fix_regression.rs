//! Regression tests for audit fixes #1–#18
//!
//! Each test targets a specific bug that was found and fixed.
//! If any of these tests fail, the corresponding fix has regressed.

// ===========================================================================
// Fix #1: align_up overflow protection (utils.rs)
// ===========================================================================

mod fix_01_align_up_overflow {
    use nebula_memory::utils::{align_up, checked_align_up};

    #[test]
    fn align_up_normal_cases() {
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
        assert_eq!(align_up(0, 4), 0);
        assert_eq!(align_up(1, 1), 1);
    }

    #[test]
    fn align_up_near_max_does_not_panic() {
        // Before fix: would overflow and panic in debug
        let _ = align_up(usize::MAX, 8);
        let _ = align_up(usize::MAX - 1, 16);
    }

    #[test]
    fn checked_align_up_returns_none_on_overflow() {
        assert_eq!(checked_align_up(usize::MAX, 8), None);
        assert_eq!(checked_align_up(usize::MAX - 3, 8), None);
    }

    #[test]
    fn checked_align_up_returns_some_for_valid_inputs() {
        assert_eq!(checked_align_up(7, 8), Some(8));
        assert_eq!(checked_align_up(8, 8), Some(8));
        assert_eq!(checked_align_up(0, 4096), Some(0));
    }
}

// ===========================================================================
// Fix #2: arena — no unbounded recursion, checked overflow
// ===========================================================================

mod fix_02_arena_no_recursion {
    use nebula_memory::arena::{Arena, ArenaConfig};

    #[test]
    fn arena_does_not_stack_overflow_on_many_allocs() {
        let config = ArenaConfig::new().with_initial_size(64);
        let arena = Arena::new(config);
        for i in 0..500 {
            let _ = arena.alloc(i as u64);
        }
    }
}

// ===========================================================================
// Fix #4: thread_safe arena CAS uses checked arithmetic
// ===========================================================================

mod fix_04_thread_safe_checked {
    use nebula_memory::arena::{ArenaConfig, ThreadSafeArena};

    #[test]
    fn thread_safe_arena_many_allocs() {
        let config = ArenaConfig::new().with_initial_size(1024);
        let arena = ThreadSafeArena::new(config);
        for i in 0..200 {
            let _ = arena.alloc(i as u64);
        }
    }
}

// ===========================================================================
// Fix #5: ThreadSafePool::clear() no deadlock
// ===========================================================================

mod fix_05_pool_clear_no_deadlock {
    use nebula_memory::pool::{Poolable, ThreadSafePool};
    use std::sync::Arc;

    #[derive(Debug)]
    struct Item(u64);
    impl Poolable for Item {
        fn reset(&mut self) {
            self.0 = 0;
        }
    }

    #[test]
    fn clear_completes_without_deadlock() {
        let pool = Arc::new(ThreadSafePool::new(10, || Item(42)));
        for _ in 0..5 {
            let _ = pool.get().unwrap();
        }
        let p = Arc::clone(&pool);
        let handle = std::thread::spawn(move || p.clear());
        assert!(handle.join().is_ok(), "clear() should not deadlock");
    }

    #[test]
    fn clear_then_get_works() {
        let pool = ThreadSafePool::new(5, || Item(42));
        let _ = pool.get().unwrap();
        pool.clear();
        // After clear, pool creates a fresh item via factory
        let item = pool.get().unwrap();
        assert_eq!(item.0, 42, "factory should produce new item after clear");
    }
}

// ===========================================================================
// Fix #6: stack try_pop CAS
// ===========================================================================

mod fix_06_stack_try_pop {
    use nebula_memory::allocator::Allocator;
    use nebula_memory::allocator::stack::StackAllocator;
    use std::alloc::Layout;

    #[test]
    fn pop_non_top_returns_false() {
        let allocator = StackAllocator::new(4096).unwrap();
        let layout = Layout::new::<u64>();

        unsafe {
            let slice1 = allocator.allocate(layout).unwrap();
            let slice2 = allocator.allocate(layout).unwrap();

            let p1 = std::ptr::NonNull::new(slice1.as_ptr() as *mut u8).unwrap();
            let p2 = std::ptr::NonNull::new(slice2.as_ptr() as *mut u8).unwrap();

            // Pop non-top → false
            assert!(!allocator.try_pop(p1, layout));

            // Pop top → true (with CAS, not bare store)
            assert!(allocator.try_pop(p2, layout));
        }
    }
}

// ===========================================================================
// Fix #7: CellCursor removed — BumpAllocator always uses AtomicCursor
// ===========================================================================

mod fix_07_bump_always_atomic {
    use nebula_memory::allocator::Allocator;
    use nebula_memory::allocator::bump::BumpAllocator;

    #[test]
    fn bump_non_threadsafe_works() {
        let allocator = BumpAllocator::new(4096).unwrap();
        let layout = std::alloc::Layout::new::<u64>();
        let ptr = unsafe { allocator.allocate(layout) };
        assert!(ptr.is_ok());
    }
}

// ===========================================================================
// Fix #8: bump internal_restore records stats correctly
// ===========================================================================

mod fix_08_bump_restore_stats {
    use nebula_memory::allocator::Allocator;
    use nebula_memory::allocator::bump::BumpAllocator;

    #[test]
    fn restore_checkpoint_resets_used() {
        let allocator = BumpAllocator::new(4096).unwrap();
        let checkpoint = allocator.checkpoint();

        let layout = std::alloc::Layout::new::<[u64; 10]>();
        unsafe {
            let _ = allocator.allocate(layout).unwrap();
        }
        assert!(allocator.used() > 0);

        allocator.restore(checkpoint).unwrap();
        assert_eq!(allocator.used(), 0);
    }
}

// ===========================================================================
// Fix #10: budget parent release propagation
// ===========================================================================

mod fix_10_budget_release {
    use nebula_memory::budget::{create_budget, create_child_budget};

    #[test]
    fn over_release_does_not_corrupt_parent() {
        let parent = create_budget("parent", 1000);
        let child = create_child_budget("child", 500, parent.clone());

        child.request_memory(100).unwrap();
        assert_eq!(parent.used(), 100);

        // Over-release: release more than allocated
        child.release_memory(300);
        assert_eq!(child.used(), 0);
        // Parent should only have released 100, not 300
        assert_eq!(parent.used(), 0);
    }

    #[test]
    fn double_release_does_not_go_negative() {
        let parent = create_budget("parent", 1000);
        let child = create_child_budget("child", 500, parent.clone());

        child.request_memory(50).unwrap();
        child.release_memory(50);
        child.release_memory(50); // already at 0

        assert_eq!(child.used(), 0);
        assert_eq!(parent.used(), 0);
    }
}

// ===========================================================================
// Fix #11: zero-size alloc returns dangling pointer
// ===========================================================================

mod fix_11_zero_size_alloc {
    use nebula_memory::arena::Arena;

    #[test]
    fn zero_size_alloc_does_not_alias_next() {
        let arena = Arena::with_capacity(4096);

        let zst_ptr = arena.alloc_bytes_aligned(0, 1).unwrap();
        let real = arena.alloc(42u64).unwrap() as *mut u64 as *mut u8;

        assert_ne!(zst_ptr, real);
    }
}

// ===========================================================================
// Fix #12: reset with zero_memory clears ALL chunks
// ===========================================================================

mod fix_12_reset_all_chunks {
    use nebula_memory::arena::{Arena, ArenaConfig};

    #[test]
    fn reset_with_zero_memory_then_reuse() {
        let config = ArenaConfig::new()
            .with_initial_size(64)
            .with_zero_memory(true);
        let mut arena = Arena::new(config);

        // Force multiple chunks
        for i in 0..100u64 {
            let _ = arena.alloc(i);
        }
        arena.reset();

        let val = arena.alloc(42u64).unwrap();
        assert_eq!(*val, 42);
    }
}

// ===========================================================================
// Fix #13: arena generation prevents stale positions
// ===========================================================================

mod fix_13_generation {
    use nebula_memory::arena::Arena;

    #[test]
    fn stale_position_rejected_after_reset() {
        let mut arena = Arena::with_capacity(4096);

        let _ = arena.alloc(1u64).unwrap();
        let stale = arena.current_position();
        arena.reset();

        assert!(
            arena.reset_to_position(stale).is_err(),
            "stale position should be rejected"
        );
    }

    #[test]
    fn current_generation_position_accepted() {
        let mut arena = Arena::with_capacity(4096);

        let _ = arena.alloc(1u64).unwrap();
        let pos = arena.current_position();
        let _ = arena.alloc(2u64).unwrap();

        assert!(arena.reset_to_position(pos).is_ok());
    }
}
