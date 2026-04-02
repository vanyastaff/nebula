//! Allocation-focused benchmarks for metrics extension
//!
//! Isolates the allocation cost of snapshot_map() and identifies redundant cloning.

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_memory::extensions::metrics::{MemoryMetric, MetricType, MetricsExtension};
use std::hint::black_box;

fn make_metric(name: &str) -> MemoryMetric {
    MemoryMetric::new(name, "description text", MetricType::Counter, "ops")
        .with_label("component", "memory")
        .with_label("stage", "bench")
        .with_label("instance", "01")
}

/// Measures ONLY snapshot_map() allocation cost, nothing else
fn bench_snapshot_allocation_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_allocation_cost");
    group.sample_size(100); // fewer samples, focus on stddev of allocs

    for &count in &[10, 100, 1000, 4096] {
        group.bench_with_input(BenchmarkId::new("snapshot_only", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let ext = MetricsExtension::new_noop();
                    for i in 0..n {
                        ext.register_metric(make_metric(&format!("metric_{:05}", i)))
                            .expect("register");
                    }
                    ext
                },
                |ext| {
                    // ISOLATED: only measure snapshot_map() calls
                    for _ in 0..10 {
                        let snapshot = ext.metrics_snapshot();
                        black_box(snapshot);
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Register unique metrics + single snapshot at end
/// Metric: how many allocations happen during snapshot?
fn bench_register_then_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("register_then_snapshot");

    for &count in &[128, 1024, 4096] {
        group.bench_with_input(
            BenchmarkId::new("full_lifecycle", count),
            &count,
            |b, &n| {
                b.iter_batched(
                    || MetricsExtension::new_noop(),
                    |ext| {
                        // Register n unique metrics
                        for i in 0..n {
                            ext.register_metric(make_metric(&format!("metric_{:05}", i)))
                                .expect("register");
                        }
                        // Single snapshot at the end
                        let map = ext.metrics_snapshot();
                        black_box(map)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Replace same metric name n times + snapshot
/// Measures: clone cost in to_metric() per update
fn bench_repeated_replacement_then_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("repeated_replacement");
    group.sample_size(50);

    for &count in &[128, 1024, 4096] {
        group.bench_with_input(BenchmarkId::new("replacements", count), &count, |b, &n| {
            b.iter_batched(
                || MetricsExtension::new_noop(),
                |ext| {
                    // Replace same metric name n times
                    for i in 0..n {
                        let metric = make_metric("mem.allocation.total")
                            .with_label("iteration", format!("{:05}", i));
                        ext.register_metric(metric).expect("register");
                    }
                    let map = ext.metrics_snapshot();
                    black_box(map)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Unregister all + snapshot
/// Measures: snapshot cost on empty set (baseline)
fn bench_empty_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("empty_snapshot");

    for &count in &[128, 1024, 4096] {
        group.bench_with_input(BenchmarkId::new("empty", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let ext = MetricsExtension::new_noop();
                    let names: Vec<_> = (0..n).map(|i| format!("metric_{:05}", i)).collect();
                    for name in &names {
                        ext.register_metric(make_metric(name)).expect("register");
                    }
                    // Remove everything
                    for name in &names {
                        ext.unregister_metric(name);
                    }
                    (ext, names)
                },
                |(ext, _names)| {
                    let map = ext.metrics_snapshot();
                    black_box(map)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Measure metrics_iter() zero-allocation path vs snapshot_map()
fn bench_zero_alloc_iter_vs_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("zero_alloc_iter_comparison");
    group.sample_size(50);

    for &count in &[100, 1000] {
        group.bench_with_input(
            BenchmarkId::new("snapshot_full_clone", count),
            &count,
            |b, &n| {
                b.iter_batched(
                    || {
                        let ext = MetricsExtension::new_noop();
                        for i in 0..n {
                            ext.register_metric(make_metric(&format!("metric_{:05}", i)))
                                .expect("register");
                        }
                        ext
                    },
                    |ext| {
                        // Full clone of all metrics
                        let map = ext.metrics_snapshot();
                        black_box(map)
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("iter_zero_alloc", count),
            &count,
            |b, &n| {
                b.iter_batched(
                    || {
                        let ext = MetricsExtension::new_noop();
                        for i in 0..n {
                            ext.register_metric(make_metric(&format!("metric_{:05}", i)))
                                .expect("register");
                        }
                        ext
                    },
                    |ext| {
                        // Zero-allocation iterator
                        let mut counter = 0;
                        ext.metrics_iter(|_name, _stored| {
                            counter += 1;
                        });
                        black_box(counter)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Hot path: repeated snapshots after initial registration
/// Metric: snapshot() cost when metrics don't change
fn bench_hot_path_repeated_snapshots(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_path_repeated_snapshots");
    group.sample_size(100);

    let registration_counts = &[10, 100];
    let snapshot_counts = &[100, 1000];

    for &regs in registration_counts {
        for &snaps in snapshot_counts {
            group.bench_with_input(
                BenchmarkId::new(
                    "register_snapshot_pattern",
                    format!("{}_regs_x_{}_snaps", regs, snaps),
                ),
                &(regs, snaps),
                |b, &(num_regs, num_snaps)| {
                    b.iter_batched(
                        || {
                            let ext = MetricsExtension::new_noop();
                            for i in 0..num_regs {
                                ext.register_metric(make_metric(&format!("metric_{:02}", i)))
                                    .expect("register");
                            }
                            ext
                        },
                        |ext| {
                            for _ in 0..num_snaps {
                                let map = ext.metrics_snapshot();
                                black_box(map);
                            }
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_snapshot_allocation_cost,
    bench_register_then_snapshot,
    bench_repeated_replacement_then_snapshot,
    bench_empty_snapshot,
    bench_hot_path_repeated_snapshots,
    bench_zero_alloc_iter_vs_snapshot
);
criterion_main!(benches);
