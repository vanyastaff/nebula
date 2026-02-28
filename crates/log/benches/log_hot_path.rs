use criterion::{Criterion, criterion_group, criterion_main};
use nebula_log::observability::{ObservabilityEvent, emit_event, shutdown_hooks};

struct BenchEvent;

impl ObservabilityEvent for BenchEvent {
    fn name(&self) -> &str {
        "bench_event"
    }
}

fn bench_emit_path(c: &mut Criterion) {
    shutdown_hooks();
    c.bench_function("log_emit_event_no_hooks", |b| {
        b.iter(|| emit_event(&BenchEvent));
    });
}

fn bench_context_snapshot(c: &mut Criterion) {
    c.bench_function("log_context_snapshot", |b| {
        b.iter(nebula_log::observability::current_contexts);
    });
}

criterion_group!(log_hot_path, bench_emit_path, bench_context_snapshot);
criterion_main!(log_hot_path);
