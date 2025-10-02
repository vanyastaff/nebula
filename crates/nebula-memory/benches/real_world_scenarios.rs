//! Real-world scenario benchmarks
//!
//! Benchmarks that simulate actual usage patterns

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use nebula_memory::allocator::{Allocator, BumpAllocator, PoolAllocator, PoolConfig};
use nebula_memory::core::traits::Resettable;
use std::alloc::Layout;

/// Simulate request/response cycle (allocate, use, deallocate)
fn bench_request_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_response");
    group.throughput(Throughput::Elements(1));

    // Bump allocator with reset
    group.bench_function("bump_with_reset", |b| {
        let allocator = BumpAllocator::new(64 * 1024).unwrap();
        let layout = Layout::from_size_align(256, 8).unwrap();

        b.iter(|| unsafe {
            // Simulate allocating request data
            let req = allocator.allocate(layout).unwrap();
            std::ptr::write_bytes(req.cast::<u8>().as_ptr(), 0x42, 256);

            // Simulate allocating response data
            let resp = allocator.allocate(layout).unwrap();
            std::ptr::write_bytes(resp.cast::<u8>().as_ptr(), 0x24, 256);

            black_box((req, resp));

            // Reset for next request
            allocator.reset();
        });
    });

    // Pool allocator with reuse
    group.bench_function("pool_with_reuse", |b| {
        let allocator = PoolAllocator::with_config(256, 8, 64, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(256, 8).unwrap();

        b.iter(|| unsafe {
            // Allocate request
            let req = allocator.allocate(layout).unwrap();
            std::ptr::write_bytes(req.cast::<u8>().as_ptr(), 0x42, 256);

            // Allocate response
            let resp = allocator.allocate(layout).unwrap();
            std::ptr::write_bytes(resp.cast::<u8>().as_ptr(), 0x24, 256);

            // Deallocate (return to pool)
            allocator.deallocate(req.cast(), layout);
            allocator.deallocate(resp.cast(), layout);

            black_box((req, resp));
        });
    });

    group.finish();
}

/// Simulate temporary buffer allocations (common in parsing)
fn bench_temporary_buffers(c: &mut Criterion) {
    let mut group = c.benchmark_group("temporary_buffers");

    group.bench_function("bump_temp_buffers", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();

        b.iter(|| unsafe {
            // Allocate various temporary buffers
            let buf1 = allocator.allocate(Layout::from_size_align(512, 8).unwrap()).unwrap();
            let buf2 = allocator.allocate(Layout::from_size_align(1024, 8).unwrap()).unwrap();
            let buf3 = allocator.allocate(Layout::from_size_align(256, 8).unwrap()).unwrap();

            // Use buffers
            std::ptr::write_bytes(buf1.cast::<u8>().as_ptr(), 1, 512);
            std::ptr::write_bytes(buf2.cast::<u8>().as_ptr(), 2, 1024);
            std::ptr::write_bytes(buf3.cast::<u8>().as_ptr(), 3, 256);

            black_box((buf1, buf2, buf3));

            // Reset at end of operation
            allocator.reset();
        });
    });

    group.finish();
}

/// Simulate object creation/destruction patterns
fn bench_object_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("object_lifecycle");
    group.throughput(Throughput::Elements(10));

    group.bench_function("pool_objects", |b| {
        let allocator = PoolAllocator::with_config(128, 8, 256, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(128, 8).unwrap();

        b.iter(|| unsafe {
            let mut objects = Vec::with_capacity(10);

            // Create 10 objects
            for i in 0..10 {
                let obj = allocator.allocate(layout).unwrap();
                std::ptr::write_bytes(obj.cast::<u8>().as_ptr(), i as u8, 128);
                objects.push(obj);
            }

            // Destroy objects
            for obj in objects {
                allocator.deallocate(obj.cast(), layout);
            }
        });
    });

    group.finish();
}

/// Simulate arena pattern - allocate many small objects, reset all at once
fn bench_arena_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_pattern");
    group.throughput(Throughput::Elements(100));

    group.bench_function("bump_arena", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();
        let layout = Layout::from_size_align(32, 8).unwrap();

        b.iter(|| unsafe {
            // Allocate many small objects
            for i in 0..100 {
                let obj = allocator.allocate(layout).unwrap();
                std::ptr::write_bytes(obj.cast::<u8>().as_ptr(), i as u8, 32);
                black_box(obj);
            }

            // Reset arena (fast bulk deallocation)
            allocator.reset();
        });
    });

    group.finish();
}

/// Benchmark mixed allocation sizes (realistic workload)
fn bench_mixed_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_sizes");

    group.bench_function("bump_mixed", |b| {
        let allocator = BumpAllocator::new(1024 * 1024).unwrap();

        b.iter(|| unsafe {
            // Small
            let s1 = allocator.allocate(Layout::from_size_align(16, 8).unwrap()).unwrap();
            let s2 = allocator.allocate(Layout::from_size_align(32, 8).unwrap()).unwrap();

            // Medium
            let m1 = allocator.allocate(Layout::from_size_align(256, 8).unwrap()).unwrap();
            let m2 = allocator.allocate(Layout::from_size_align(512, 8).unwrap()).unwrap();

            // Large
            let l1 = allocator.allocate(Layout::from_size_align(4096, 8).unwrap()).unwrap();

            black_box((s1, s2, m1, m2, l1));

            allocator.reset();
        });
    });

    group.finish();
}

/// Benchmark high-frequency allocations (stress test)
fn bench_high_frequency(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_frequency");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("pool_1000_allocs", |b| {
        let allocator = PoolAllocator::with_config(64, 8, 2048, PoolConfig::default()).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            for _ in 0..1000 {
                let ptr = allocator.allocate(layout).unwrap();
                allocator.deallocate(ptr.cast(), layout);
            }
        });
    });

    group.bench_function("bump_1000_allocs", |b| {
        let allocator = BumpAllocator::new(10 * 1024 * 1024).unwrap();
        let layout = Layout::from_size_align(64, 8).unwrap();

        b.iter(|| unsafe {
            for _ in 0..1000 {
                let ptr = allocator.allocate(layout).unwrap();
                black_box(ptr);
            }
            allocator.reset();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_request_response,
    bench_temporary_buffers,
    bench_object_lifecycle,
    bench_arena_pattern,
    bench_mixed_sizes,
    bench_high_frequency
);

criterion_main!(benches);
