//! Arena-based bulk allocation with scoped lifetimes
//!
//! Demonstrates `Arena` and `TypedArena` — allocate many objects fast,
//! then free everything at once. Ideal for batch-scoped data in workflows.

use nebula_memory::arena::{Arena, ArenaConfig, ArenaScope, TypedArena};

fn main() {
    // === Example 1: Basic arena allocation ===
    println!("=== 1. Basic Arena ===");

    let mut arena = Arena::with_capacity(4096);

    let x = arena.alloc(42i32).unwrap();
    let msg = arena.alloc_str("hello from the arena").unwrap();
    let slice = arena.alloc_slice(&[1u64, 2, 3, 4, 5]).unwrap();

    println!("int:   {x}");
    println!("str:   {msg}");
    println!("slice: {slice:?}");

    let stats = arena.stats();
    println!(
        "Used: {} bytes, Allocated: {} bytes",
        stats.bytes_used(),
        stats.bytes_allocated()
    );

    // Reset frees all at once (no per-item dealloc!)
    arena.reset();
    println!("After reset: used={} bytes", arena.stats().bytes_used());

    // === Example 2: Preset configs ===
    println!("\n=== 2. Preset configurations ===");

    let presets = [
        ("production", Arena::production(8192)),
        ("debug", Arena::debug(8192)),
        ("performance", Arena::performance(8192)),
    ];

    for (name, mut a) in presets {
        let _ = a.alloc_slice(&[0u8; 1000]);
        println!("{name:>12}: used={} bytes", a.stats().bytes_used());
        a.reset();
    }

    // === Example 3: Position save/restore ===
    println!("\n=== 3. Save and restore position ===");

    let mut arena = Arena::with_capacity(4096);

    let _ = arena.alloc(1u64).unwrap();
    let _ = arena.alloc(2u64).unwrap();
    let checkpoint = arena.current_position();
    println!("Checkpoint at {} bytes used", arena.stats().bytes_used());

    // Allocate more
    let _ = arena.alloc_slice(&[0u8; 256]).unwrap();
    println!(
        "After more allocs: {} bytes used",
        arena.stats().bytes_used()
    );

    // Restore to checkpoint — frees everything allocated after it
    arena.reset_to_position(checkpoint).unwrap();
    println!("After restore: {} bytes used", arena.stats().bytes_used());

    // === Example 4: ArenaScope (RAII) ===
    println!("\n=== 4. ArenaScope (auto-reset on drop) ===");

    let mut scope = ArenaScope::with_default();
    let greeting = scope.alloc_str("scoped string").unwrap();
    println!("In scope: {greeting}");
    println!("Scope used: {} bytes", scope.arena().stats().bytes_used());

    scope.reset();
    println!(
        "After scope reset: {} bytes",
        scope.arena().stats().bytes_used()
    );

    // === Example 5: TypedArena for homogeneous data ===
    println!("\n=== 5. TypedArena<Point> ===");

    #[derive(Debug, Copy, Clone)]
    #[allow(dead_code)]
    struct Point {
        x: f64,
        y: f64,
    }

    let mut typed = TypedArena::<Point>::with_capacity(100);

    let p1 = typed.alloc(Point { x: 1.0, y: 2.0 }).unwrap();
    let p2 = typed.alloc(Point { x: 3.0, y: 4.0 }).unwrap();
    println!("p1={p1:?}, p2={p2:?}");

    // Bulk allocate from slice
    let points = typed
        .alloc_slice(&[
            Point { x: 5.0, y: 6.0 },
            Point { x: 7.0, y: 8.0 },
            Point { x: 9.0, y: 10.0 },
        ])
        .unwrap();
    println!("Bulk allocated {} points", points.len());

    let snap = typed.stats_snapshot();
    println!(
        "TypedArena: {} bytes used, {} allocs",
        snap.bytes_used, snap.allocations
    );

    typed.reset();
    println!("After reset: {} bytes used", typed.stats().bytes_used());

    // === Example 6: Custom config ===
    println!("\n=== 6. Custom ArenaConfig ===");

    let config = ArenaConfig::new()
        .with_initial_size(16384)
        .with_growth_factor(2.0)
        .with_zero_memory(true);

    let mut arena = Arena::new(config);
    let buf = arena.alloc_slice(&[0u8; 8192]).unwrap();
    println!("Allocated {} bytes with custom config", buf.len());
    arena.reset();
}
