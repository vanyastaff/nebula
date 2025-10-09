//! Simple performance benchmarks for allocators
//!
//! This demonstrates the performance characteristics of different allocators.
//! For comprehensive benchmarks, use `cargo bench`.

use nebula_memory::allocator::{BumpAllocator, PoolAllocator, StackAllocator, TypedAllocator};
use nebula_memory::prelude::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== nebula-memory Performance Benchmarks ===\n");

    println!("Platform: {}", std::env::consts::ARCH);
    println!("OS: {}\n", std::env::consts::OS);

    // Benchmark 1: Single allocation
    println!("1. Single 64-byte Allocation:");
    benchmark_single_alloc()?;
    println!();

    // Benchmark 2: Batch allocations
    println!("2. Batch Allocations (1000x 64 bytes):");
    benchmark_batch_alloc()?;
    println!();

    // Benchmark 3: Reuse pattern
    println!("3. Allocation/Deallocation Reuse (1000 cycles):");
    benchmark_reuse_pattern()?;
    println!();

    // Benchmark 4: Arena reset
    println!("4. Arena Reset Performance:");
    benchmark_reset()?;
    println!();

    Ok(())
}

fn benchmark_single_alloc() -> Result<(), Box<dyn std::error::Error>> {
    const ITERATIONS: usize = 10_000;

    // System allocator
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let b = Box::new([0u8; 64]);
        std::hint::black_box(b);
    }
    let system_time = start.elapsed();
    println!("  System allocator:  {:>8.2?} ({:.2}ns per alloc)",
        system_time, system_time.as_nanos() as f64 / ITERATIONS as f64);

    // Bump allocator
    let allocator = BumpAllocator::new(1024 * 1024)?;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let ptr = unsafe { allocator.alloc::<[u8; 64]>()? };
        std::hint::black_box(ptr);
    }
    let bump_time = start.elapsed();
    println!("  Bump allocator:    {:>8.2?} ({:.2}ns per alloc) - {:.1}x faster",
        bump_time,
        bump_time.as_nanos() as f64 / ITERATIONS as f64,
        system_time.as_nanos() as f64 / bump_time.as_nanos() as f64);

    // Pool allocator
    let pool = PoolAllocator::new(64, 8, ITERATIONS)?;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let ptr = unsafe { pool.alloc::<[u8; 64]>()? };
        std::hint::black_box(ptr);
        unsafe { pool.dealloc(ptr); }
    }
    let pool_time = start.elapsed();
    println!("  Pool allocator:    {:>8.2?} ({:.2}ns per alloc) - {:.1}x faster",
        pool_time,
        pool_time.as_nanos() as f64 / ITERATIONS as f64,
        system_time.as_nanos() as f64 / pool_time.as_nanos() as f64);

    // Stack allocator
    let stack = StackAllocator::new(1024 * 1024)?;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let ptr = unsafe { stack.alloc::<[u8; 64]>()? };
        std::hint::black_box(ptr);
    }
    let stack_time = start.elapsed();
    println!("  Stack allocator:   {:>8.2?} ({:.2}ns per alloc) - {:.1}x faster",
        stack_time,
        stack_time.as_nanos() as f64 / ITERATIONS as f64,
        system_time.as_nanos() as f64 / stack_time.as_nanos() as f64);

    Ok(())
}

fn benchmark_batch_alloc() -> Result<(), Box<dyn std::error::Error>> {
    const COUNT: usize = 1_000;
    const ITERATIONS: usize = 100;

    // System allocator
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let mut boxes = Vec::with_capacity(COUNT);
        for _ in 0..COUNT {
            boxes.push(Box::new([0u8; 64]));
        }
        std::hint::black_box(boxes);
    }
    let system_time = start.elapsed();
    println!("  System allocator:  {:>8.2?} ({:.2}µs per batch)",
        system_time, system_time.as_micros() as f64 / ITERATIONS as f64);

    // Bump allocator
    let allocator = BumpAllocator::new(10 * 1024 * 1024)?;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        for _ in 0..COUNT {
            let ptr = unsafe { allocator.alloc::<[u8; 64]>()? };
            std::hint::black_box(ptr);
        }
        allocator.reset();
    }
    let bump_time = start.elapsed();
    println!("  Bump allocator:    {:>8.2?} ({:.2}µs per batch) - {:.1}x faster",
        bump_time,
        bump_time.as_micros() as f64 / ITERATIONS as f64,
        system_time.as_micros() as f64 / bump_time.as_micros() as f64);

    Ok(())
}

fn benchmark_reuse_pattern() -> Result<(), Box<dyn std::error::Error>> {
    const CYCLES: usize = 1_000;

    // Pool allocator (designed for reuse)
    let pool = PoolAllocator::new(64, 8, 100)?;
    let start = Instant::now();
    for _ in 0..CYCLES {
        let ptr = unsafe { pool.alloc::<[u8; 64]>()? };
        std::hint::black_box(ptr);
        unsafe { pool.dealloc(ptr); }
    }
    let pool_time = start.elapsed();
    println!("  Pool allocator:    {:>8.2?} ({:.2}ns per cycle)",
        pool_time, pool_time.as_nanos() as f64 / CYCLES as f64);

    // System allocator (for comparison)
    let start = Instant::now();
    for _ in 0..CYCLES {
        let b = Box::new([0u8; 64]);
        std::hint::black_box(b);
    }
    let system_time = start.elapsed();
    println!("  System allocator:  {:>8.2?} ({:.2}ns per cycle) - {:.1}x slower",
        system_time,
        system_time.as_nanos() as f64 / CYCLES as f64,
        system_time.as_nanos() as f64 / pool_time.as_nanos() as f64);

    Ok(())
}

fn benchmark_reset() -> Result<(), Box<dyn std::error::Error>> {
    const ITERATIONS: usize = 10_000;

    let allocator = BumpAllocator::new(1024 * 1024)?;

    // Allocate some memory
    for _ in 0..100 {
        unsafe { allocator.alloc::<[u8; 64]>()? };
    }

    // Benchmark reset
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        allocator.reset();
    }
    let reset_time = start.elapsed();
    println!("  Reset time:        {:>8.2?} ({:.2}ns per reset)",
        reset_time, reset_time.as_nanos() as f64 / ITERATIONS as f64);
    println!("  → O(1) operation regardless of allocated memory!");

    Ok(())
}
