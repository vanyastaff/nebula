//! Comprehensive allocator benchmarks
//!
//! Compares performance of different allocators across various workloads.
//! Uses checkpoint/restore for BumpAllocator so each iteration starts fresh
//! without running out of capacity.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_memory::allocator::{
    Allocator, BumpAllocator, PoolAllocator, PoolConfig, StackAllocator, StackConfig,
};
use std::alloc::Layout;
use std::hint::black_box;

// ---------------------------------------------------------------------------
// Single allocation/deallocation cycle
// ---------------------------------------------------------------------------

fn bench_single_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_allocation");

    // Bump: checkpoint → allocate → restore (the real bump pattern)
    group.bench_function("bump_64b", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let cp = allocator.checkpoint();
            let ptr = allocator.allocate(layout).unwrap();
            black_box(ptr);
            let _ = allocator.restore(cp);
        });
    });

    // Pool: allocate → return to pool
    group.bench_function("pool_64b", |b| {
        let allocator = PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            black_box(ptr);
            allocator.deallocate(ptr.cast(), layout);
        });
    });

    // Stack: allocate → LIFO deallocate
    group.bench_function("stack_64b", |b| {
        let allocator =
            StackAllocator::with_config(1024 * 1024, StackConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            black_box(ptr);
            allocator.deallocate(ptr.cast(), layout);
        });
    });

    // System allocator — baseline
    group.bench_function("system_64b", |b| {
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let ptr = std::alloc::alloc(layout);
            black_box(ptr);
            std::alloc::dealloc(ptr, layout);
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Batch allocations (100 ops)
// ---------------------------------------------------------------------------

fn bench_batch_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_allocations");
    group.throughput(Throughput::Elements(100));

    // Bump: allocate 100, then restore once — the natural bulk-alloc pattern
    group.bench_function("bump_100x64b", |b| {
        let allocator = BumpAllocator::new(16 * 1024 * 1024).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let cp = allocator.checkpoint();
            for _ in 0..100 {
                let ptr = allocator.allocate(layout).unwrap();
                black_box(ptr);
            }
            let _ = allocator.restore(cp);
        });
    });

    // Pool: allocate 100, then return all — tests free-list recycling
    group.bench_function("pool_100x64b", |b| {
        let allocator =
            PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let mut ptrs = Vec::with_capacity(100);
            for _ in 0..100 {
                ptrs.push(allocator.allocate(layout).unwrap());
            }
            for ptr in ptrs {
                allocator.deallocate(ptr.cast(), layout);
            }
        });
    });

    // Stack: allocate 100 (LIFO), then deallocate in reverse
    group.bench_function("stack_100x64b", |b| {
        let allocator =
            StackAllocator::with_config(1024 * 1024, StackConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let mut ptrs = Vec::with_capacity(100);
            for _ in 0..100 {
                ptrs.push(allocator.allocate(layout).unwrap());
            }
            for ptr in ptrs.into_iter().rev() {
                allocator.deallocate(ptr.cast(), layout);
            }
        });
    });

    // System baseline for batch
    group.bench_function("system_100x64b", |b| {
        let layout = Layout::from_size_align(64, 8).unwrap();
        b.iter(|| unsafe {
            let mut ptrs = Vec::with_capacity(100);
            for _ in 0..100 {
                ptrs.push(std::alloc::alloc(layout));
            }
            for ptr in ptrs {
                std::alloc::dealloc(ptr, layout);
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Performance across different allocation sizes
// ---------------------------------------------------------------------------

fn bench_allocation_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_sizes");

    for &size in &[16_usize, 64, 256, 1024, 4096] {
        let layout = Layout::from_size_align(size, 8).unwrap();

        group.bench_with_input(BenchmarkId::new("bump", size), &size, |b, _| {
            let allocator = BumpAllocator::new(64 * 1024 * 1024).unwrap();
            b.iter(|| unsafe {
                let cp = allocator.checkpoint();
                let ptr = allocator.allocate(layout).unwrap();
                black_box(ptr);
                let _ = allocator.restore(cp);
            });
        });

        group.bench_with_input(BenchmarkId::new("pool", size), &size, |b, _| {
            let allocator =
                PoolAllocator::with_config(size, 8, 1024, PoolConfig::default()).unwrap();
            b.iter(|| unsafe {
                let ptr = allocator.allocate(layout).unwrap();
                black_box(ptr);
                allocator.deallocate(ptr.cast(), layout);
            });
        });

        group.bench_with_input(BenchmarkId::new("system", size), &size, |b, _| {
            b.iter(|| unsafe {
                let ptr = std::alloc::alloc(layout);
                black_box(ptr);
                std::alloc::dealloc(ptr, layout);
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Memory reuse efficiency
// ---------------------------------------------------------------------------

fn bench_memory_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_reuse");

    // Pool should reuse the same block without hitting the allocator internals
    group.bench_function("pool_reuse_128b", |b| {
        let allocator =
            PoolAllocator::with_config(128, 8, 256, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(128, 8).unwrap();
        b.iter(|| unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr1.cast(), layout);
            let ptr2 = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr2.cast(), layout);
            black_box((ptr1, ptr2));
        });
    });

    // Bump: alloc → restore → alloc → restore (no reuse — measures raw alloc cost)
    group.bench_function("bump_no_reuse_128b", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(128, 8).unwrap();
        b.iter(|| unsafe {
            let cp1 = allocator.checkpoint();
            let ptr1 = allocator.allocate(layout).unwrap();
            let _ = allocator.restore(cp1);

            let cp2 = allocator.checkpoint();
            let ptr2 = allocator.allocate(layout).unwrap();
            let _ = allocator.restore(cp2);

            black_box((ptr1, ptr2));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Large allocations (1 MB blocks)
// ---------------------------------------------------------------------------

fn bench_large_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_allocations");
    group.sample_size(50);

    group.bench_function("bump_1mb", |b| {
        let allocator = BumpAllocator::new(64 * 1024 * 1024).unwrap();
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();
        b.iter(|| unsafe {
            let cp = allocator.checkpoint();
            let ptr = allocator.allocate(layout).unwrap();
            black_box(ptr);
            let _ = allocator.restore(cp);
        });
    });

    group.bench_function("stack_1mb", |b| {
        let allocator =
            StackAllocator::with_config(64 * 1024 * 1024, StackConfig::default()).unwrap();
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();
        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            black_box(ptr);
            allocator.deallocate(ptr.cast(), layout);
        });
    });

    group.bench_function("system_1mb", |b| {
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();
        b.iter(|| unsafe {
            let ptr = std::alloc::alloc(layout);
            black_box(ptr);
            std::alloc::dealloc(ptr, layout);
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Fresh-allocator cost via iter_batched (measures creation + one alloc)
// ---------------------------------------------------------------------------

fn bench_allocator_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocator_creation");
    group.sample_size(50);

    group.bench_function("bump_new_1mb", |b| {
        b.iter_batched(
            || {},
            |_| BumpAllocator::new(1024 * 1024).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("pool_new_64b_1024slots", |b| {
        b.iter_batched(
            || {},
            |_| PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_single_allocation,
    bench_batch_allocations,
    bench_allocation_sizes,
    bench_memory_reuse,
    bench_large_allocations,
    bench_allocator_creation,
);
criterion_main!(benches);
