//! Metrics extension benchmarks
//!
//! Focused on register/update/remove paths for MetricsExtension.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_memory::extensions::metrics::{MemoryMetric, MetricType, MetricsExtension, MetricsReporter};
use std::{collections::BTreeMap, hint::black_box, sync::Arc};

struct DynNoopReporter;

impl MetricsReporter for DynNoopReporter {
    fn register_metric(&self, _metric: &MemoryMetric) -> nebula_memory::error::MemoryResult<()> {
        Ok(())
    }

    fn report_counter(
        &self,
        _name: &str,
        _value: u64,
        _labels: &BTreeMap<String, String>,
    ) -> nebula_memory::error::MemoryResult<()> {
        Ok(())
    }

    fn report_gauge(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> nebula_memory::error::MemoryResult<()> {
        Ok(())
    }

    fn report_histogram(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> nebula_memory::error::MemoryResult<()> {
        Ok(())
    }

    fn report_summary(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> nebula_memory::error::MemoryResult<()> {
        Ok(())
    }
}

fn make_metric(name: &str) -> MemoryMetric {
    MemoryMetric::new(name, "bench metric", MetricType::Counter, "ops")
        .with_label("component", "memory")
        .with_label("stage", "bench")
}

fn bench_register_unique(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_register_unique");

    for &count in &[128usize, 1024, 4096] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::new("noop_adapter", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let ext = MetricsExtension::new_noop();
                    let metrics = (0..n)
                        .map(|i| make_metric(&format!("mem.alloc.{i}")))
                        .collect::<Vec<_>>();
                    (ext, metrics)
                },
                |(ext, metrics)| {
                    for metric in metrics {
                        ext.register_metric(metric).expect("register metric");
                    }
                    black_box(ext.metrics_snapshot());
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("custom_dyn", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let ext = MetricsExtension::new_custom(Arc::new(DynNoopReporter));
                    let metrics = (0..n)
                        .map(|i| make_metric(&format!("mem.alloc.{i}")))
                        .collect::<Vec<_>>();
                    (ext, metrics)
                },
                |(ext, metrics)| {
                    for metric in metrics {
                        ext.register_metric(metric).expect("register metric");
                    }
                    black_box(ext.metrics_snapshot());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_register_replace(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_register_replace");

    for &count in &[128usize, 1024, 4096] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::new("noop_adapter", count), &count, |b, &n| {
            b.iter_batched(
                || MetricsExtension::new_noop(),
                |ext| {
                    for i in 0..n {
                        let metric = make_metric("mem.alloc.total")
                            .with_label("iteration", i.to_string());
                        ext.register_metric(metric).expect("replace metric");
                    }
                    black_box(ext.metrics_snapshot());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_unregister_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_unregister_cycle");

    for &count in &[128usize, 1024, 4096] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::new("noop_adapter", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let ext = MetricsExtension::new_noop();
                    let names = (0..n)
                        .map(|i| format!("mem.alloc.{i}"))
                        .collect::<Vec<_>>();
                    for name in &names {
                        ext.register_metric(make_metric(name)).expect("register metric");
                    }
                    (ext, names)
                },
                |(ext, names)| {
                    for name in names {
                        let removed = ext.unregister_metric(&name);
                        black_box(removed);
                    }
                    black_box(ext.metrics_snapshot());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_register_unique,
    bench_register_replace,
    bench_unregister_cycle
);
criterion_main!(benches);
