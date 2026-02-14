//! Comprehensive allocator benchmarks
//!
//! Compares performance of different allocators across various workloads

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_memory::allocator::{
    Allocator, BumpAllocator, PoolAllocator, PoolConfig, StackAllocator, StackConfig,
};
use std::alloc::Layout;
use std::hint::black_box;

/// Benchmark single allocation/deallocation cycle
fn bench_single_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_allocation");

    // Bump allocator
    group.bench_function("bump_64b", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
            black_box(ptr);
        });
    });

    // Pool allocator
    group.bench_function("pool_64b", |b| {
        let allocator = PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
            black_box(ptr);
        });
    });

    // Stack allocator
    group.bench_function("stack_64b", |b| {
        let allocator = StackAllocator::with_config(1024 * 1024, StackConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
            black_box(ptr);
        });
    });

    // System allocator (baseline)
    group.bench_function("system_64b", |b| {
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = std::alloc::alloc(layout);
            std::alloc::dealloc(ptr, layout);
            black_box(ptr);
        });
    });

    group.finish();
}

/// Benchmark batch allocations
fn bench_batch_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_allocations");
    group.throughput(Throughput::Elements(100));

    // Bump allocator
    group.bench_function("bump_100x64b", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
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

    // Pool allocator
    group.bench_function("pool_100x64b", |b| {
        let allocator = PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap();
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

    // Stack allocator
    group.bench_function("stack_100x64b", |b| {
        let allocator = StackAllocator::with_config(1024 * 1024, StackConfig::default()).unwrap();
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

    group.finish();
}

/// Benchmark different allocation sizes
fn bench_allocation_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_sizes");

    for size in [16, 64, 256, 1024, 4096].iter() {
        group.bench_with_input(BenchmarkId::new("bump", size), size, |b, &size| {
            let allocator = BumpAllocator::new(1024 * 1024).unwrap();
            let layout = Layout::from_size_align(size, 8).unwrap();

            b.iter(|| unsafe {
                let ptr = allocator.allocate(layout).unwrap();
                allocator.deallocate(ptr.cast(), layout);
                black_box(ptr);
            });
        });

        group.bench_with_input(BenchmarkId::new("pool", size), size, |b, &size| {
            let allocator =
                PoolAllocator::with_config(size, 8, 1024, PoolConfig::default()).unwrap();
            let layout = Layout::from_size_align(size, 8).unwrap();

            b.iter(|| unsafe {
                let ptr = allocator.allocate(layout).unwrap();
                allocator.deallocate(ptr.cast(), layout);
                black_box(ptr);
            });
        });
    }

    group.finish();
}

/// Benchmark allocation patterns (sequential, random, interleaved)
fn bench_allocation_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_patterns");

    // Sequential: allocate all, then deallocate all
    group.bench_function("bump_sequential", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            let mut ptrs = Vec::with_capacity(50);

            // Allocate
            for _ in 0..50 {
                ptrs.push(allocator.allocate(layout).unwrap());
            }

            // Deallocate
            for ptr in ptrs {
                allocator.deallocate(ptr.cast(), layout);
            }
        });
    });

    // Interleaved: allocate and deallocate in pairs
    group.bench_function("pool_interleaved", |b| {
        let allocator = PoolAllocator::with_config(64, 8, 1024, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            for _ in 0..50 {
                let ptr = allocator.allocate(layout).unwrap();
                allocator.deallocate(ptr.cast(), layout);
            }
        });
    });

    group.finish();
}

/// Benchmark memory reuse efficiency
fn bench_memory_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_reuse");

    // Pool allocator should excel at reuse
    group.bench_function("pool_reuse", |b| {
        let allocator = PoolAllocator::with_config(128, 8, 256, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(128, 8).unwrap();

        b.iter(|| unsafe {
            // Allocate
            let ptr1 = allocator.allocate(layout).unwrap();

            // Deallocate
            allocator.deallocate(ptr1.cast(), layout);

            // Allocate again (should reuse same block)
            let ptr2 = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr2.cast(), layout);

            black_box((ptr1, ptr2));
        });
    });

    // Bump allocator does not reuse
    group.bench_function("bump_no_reuse", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(128, 8).unwrap();

        b.iter(|| unsafe {
            let ptr1 = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr1.cast(), layout);

            let ptr2 = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr2.cast(), layout);

            black_box((ptr1, ptr2));
        });
    });

    group.finish();
}

/// Benchmark large allocations
fn bench_large_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_allocations");
    group.sample_size(50); // Fewer samples for expensive operations

    // 1MB allocations
    group.bench_function("bump_1mb", |b| {
        let allocator = BumpAllocator::new(10 * 1024 * 1024).unwrap();
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
            black_box(ptr);
        });
    });

    // 1MB stack
    group.bench_function("stack_1mb", |b| {
        let allocator =
            StackAllocator::with_config(10 * 1024 * 1024, StackConfig::default()).unwrap();
        let layout = Layout::from_size_align(1024 * 1024, 8).unwrap();

        b.iter(|| unsafe {
            let ptr = allocator.allocate(layout).unwrap();
            allocator.deallocate(ptr.cast(), layout);
            black_box(ptr);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_single_allocation,
    bench_batch_allocations,
    bench_allocation_sizes,
    bench_allocation_patterns,
    bench_memory_reuse,
    bench_large_allocations
);

criterion_main!(benches);
