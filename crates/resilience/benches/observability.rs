//! Benchmarks for observability hook throughput and fanout contention behavior
#![expect(
    clippy::excessive_nesting,
    reason = "Fanout benchmark intentionally nests spawn and await loops for load modeling"
)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::observability::{ObservabilityHook, ObservabilityHooks, PatternEvent};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct CountingHook {
    count: AtomicU64,
}

impl ObservabilityHook for CountingHook {
    fn on_event(&self, _event: &PatternEvent) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

fn sample_event() -> PatternEvent {
    PatternEvent::Failed {
        pattern: "retry".to_string(),
        operation: "bench-op".to_string(),
        error: "downstream failure".to_string(),
        duration: std::time::Duration::from_millis(2),
    }
}

fn observability_emit_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("observability/emit");

    group.bench_function("single_hook", |b| {
        let hooks = ObservabilityHooks::new().with_hook(Arc::new(CountingHook::default()));
        b.iter(|| {
            hooks.emit(&sample_event());
            black_box(())
        });
    });

    group.bench_function("fanout_4_hooks", |b| {
        let mut hooks = ObservabilityHooks::new();
        for _ in 0..4 {
            hooks = hooks.with_hook(Arc::new(CountingHook::default()));
        }

        b.iter(|| {
            hooks.emit(&sample_event());
            black_box(())
        });
    });

    group.finish();
}

fn observability_emit_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("observability/contention");
    group.sample_size(40);

    for &num_tasks in &[8usize, 32, 96] {
        group.bench_with_input(
            BenchmarkId::new("parallel_emit", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().expect("runtime");
                let mut hooks = ObservabilityHooks::new();
                for _ in 0..4 {
                    hooks = hooks.with_hook(Arc::new(CountingHook::default()));
                }
                let hooks = Arc::new(hooks);

                b.to_async(&rt).iter(|| {
                    let hooks = Arc::clone(&hooks);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let hooks = Arc::clone(&hooks);
                            handles.push(tokio::spawn(async move {
                                hooks.emit(&sample_event());
                            }));
                        }

                        for handle in handles {
                            let _ = handle.await;
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    observability_emit_throughput,
    observability_emit_contention,
);
criterion_main!(benches);
