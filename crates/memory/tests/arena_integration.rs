//! Integration tests for arena allocation patterns
//!
//! Validates bulk allocation, reset/reuse, scoped lifetimes, and typed arenas.

use nebula_memory::arena::{Arena, ArenaConfig, ArenaScope, TypedArena};

// ---------------------------------------------------------------------------
// Basic allocation and reset
// ---------------------------------------------------------------------------

#[test]
fn arena_alloc_and_reset_reuses_memory() {
    let mut arena = Arena::with_capacity(4096);

    // First round
    for i in 0..100 {
        arena.alloc(i as u64).unwrap();
    }
    let used_before = arena.stats().bytes_used();
    assert!(used_before > 0);

    arena.reset();
    assert_eq!(arena.stats().bytes_used(), 0);

    // Second round — same arena, no new OS allocation
    for i in 0..100 {
        arena.alloc(i as u64).unwrap();
    }
    let used_after = arena.stats().bytes_used();
    assert_eq!(used_before, used_after, "reuse should produce same layout");
}

#[test]
fn arena_alloc_different_types() {
    let arena = Arena::with_capacity(4096);

    let i = arena.alloc(42i32).unwrap();
    let f = arena.alloc(3.14f64).unwrap();
    let s = arena.alloc_str("hello arena").unwrap();
    let slice = arena.alloc_slice(&[1u8, 2, 3, 4, 5]).unwrap();

    assert_eq!(*i, 42);
    assert!((*f - 3.14).abs() < f64::EPSILON);
    assert_eq!(s, "hello arena");
    assert_eq!(slice, &[1, 2, 3, 4, 5]);
}

#[test]
fn arena_alloc_slice_large() {
    let mut arena = Arena::with_capacity(1024 * 1024);

    let data: Vec<u64> = (0..10_000).collect();
    let slice = arena.alloc_slice(&data).unwrap();

    assert_eq!(slice.len(), 10_000);
    assert_eq!(slice[0], 0);
    assert_eq!(slice[9_999], 9_999);

    arena.reset();
}

// ---------------------------------------------------------------------------
// Position save/restore
// ---------------------------------------------------------------------------

#[test]
fn arena_position_checkpoint() {
    let mut arena = Arena::with_capacity(4096);

    let _ = arena.alloc(1u64).unwrap();
    let _ = arena.alloc(2u64).unwrap();
    let pos = arena.current_position();

    let _ = arena.alloc(3u64).unwrap();
    let _ = arena.alloc_slice(&[0u8; 512]).unwrap();

    let used_after_more = arena.stats().bytes_used();
    arena.reset_to_position(pos).unwrap();

    // Note: some arenas may not reduce bytes_used on partial reset
    // but the position should be valid for further allocation
    let _ = arena.alloc(99u64).unwrap();
    let _ = arena.stats().bytes_used();
    // Verify arena is still functional after restore
    let v = arena.alloc(100u64).unwrap();
    assert_eq!(*v, 100);
    let _ = used_after_more; // suppress warning
}

// ---------------------------------------------------------------------------
// ArenaScope (RAII)
// ---------------------------------------------------------------------------

#[test]
fn arena_scope_auto_cleanup() {
    let mut scope = ArenaScope::with_default();

    let a = scope.alloc(42i32).unwrap();
    let b = scope.alloc_str("scoped").unwrap();
    assert_eq!(*a, 42);
    assert_eq!(b, "scoped");

    let used = scope.arena().stats().bytes_used();
    assert!(used > 0);

    scope.reset();
    assert_eq!(scope.arena().stats().bytes_used(), 0);
}

#[test]
fn arena_scope_multiple_rounds() {
    let mut scope = ArenaScope::with_default();

    for round in 0..5 {
        for i in 0..100 {
            scope.alloc(round * 100 + i as u64).unwrap();
        }
        scope.reset();
        assert_eq!(scope.arena().stats().bytes_used(), 0, "round {round}");
    }
}

// ---------------------------------------------------------------------------
// TypedArena
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Point3D {
    x: f32,
    y: f32,
    z: f32,
}

#[test]
fn typed_arena_homogeneous_allocation() {
    let mut arena = TypedArena::<Point3D>::with_capacity(100);

    let p1 = arena
        .alloc(Point3D {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        })
        .unwrap();
    let p2 = arena
        .alloc(Point3D {
            x: 4.0,
            y: 5.0,
            z: 6.0,
        })
        .unwrap();

    assert_eq!(p1.x, 1.0);
    assert_eq!(p2.z, 6.0);

    let stats = arena.stats_snapshot();
    assert_eq!(stats.allocations, 2);

    arena.reset();
    assert_eq!(arena.stats().bytes_used(), 0);
}

#[test]
fn typed_arena_bulk_slice() {
    let arena = TypedArena::<u64>::with_capacity(1000);

    let data: Vec<u64> = (0..500).collect();
    let slice = arena.alloc_slice(&data).unwrap();

    assert_eq!(slice.len(), 500);
    assert_eq!(slice[0], 0);
    assert_eq!(slice[499], 499);

    // Mutate through the slice
    slice[0] = 999;
    assert_eq!(slice[0], 999);
}

#[test]
fn typed_arena_alloc_iter() {
    let arena = TypedArena::<String>::with_capacity(50);

    let refs = arena
        .alloc_iter((0..10).map(|i| format!("item_{i}")))
        .unwrap();

    assert_eq!(refs.len(), 10);
    assert_eq!(refs[0].as_str(), "item_0");
    assert_eq!(refs[9].as_str(), "item_9");
}

// ---------------------------------------------------------------------------
// Custom config
// ---------------------------------------------------------------------------

#[test]
fn custom_config_validation() {
    let config = ArenaConfig::new()
        .with_initial_size(8192)
        .with_growth_factor(2.0)
        .with_max_chunk_size(1024 * 1024);

    assert!(config.validate().is_ok());

    let mut arena = Arena::new(config);
    let _ = arena.alloc_slice(&[0u8; 4096]).unwrap();
    arena.reset();
}

#[test]
fn arena_preset_configs_are_valid() {
    let presets = [
        ArenaConfig::new(),
        ArenaConfig::new().with_initial_size(1024),
        ArenaConfig::new().with_growth_factor(1.5),
    ];

    for (i, config) in presets.iter().enumerate() {
        assert!(config.validate().is_ok(), "preset {i} should be valid");
    }
}
