use criterion::{Criterion, criterion_group, criterion_main};
use nebula_log::observability::{
    HookPolicy, ObservabilityEvent, ObservabilityHook, emit_event, register_hook, set_hook_policy,
    shutdown_hooks,
};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct BenchEvent;

impl ObservabilityEvent for BenchEvent {
    fn name(&self) -> &str {
        "bench_event"
    }
}

struct NoopHook;

impl ObservabilityHook for NoopHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {}
}

struct CountingHook {
    count: Arc<AtomicU64>,
}

impl ObservabilityHook for CountingHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

fn bench_emit_no_hooks(c: &mut Criterion) {
    shutdown_hooks();
    set_hook_policy(HookPolicy::Inline);
    c.bench_function("log_emit_event_no_hooks_inline", |b| {
        b.iter(|| emit_event(&BenchEvent));
    });
}

fn bench_emit_with_inline_noop_hook(c: &mut Criterion) {
    shutdown_hooks();
    set_hook_policy(HookPolicy::Inline);
    register_hook(Arc::new(NoopHook));

    c.bench_function("log_emit_event_one_noop_hook_inline", |b| {
        b.iter(|| emit_event(&BenchEvent));
    });

    shutdown_hooks();
}

fn bench_emit_with_bounded_counting_hook(c: &mut Criterion) {
    shutdown_hooks();
    set_hook_policy(HookPolicy::Bounded {
        timeout_ms: 10,
        queue_capacity: 128,
    });
    register_hook(Arc::new(CountingHook {
        count: Arc::new(AtomicU64::new(0)),
    }));

    c.bench_function("log_emit_event_one_counting_hook_bounded", |b| {
        b.iter(|| emit_event(&BenchEvent));
    });

    shutdown_hooks();
}

fn bench_context_snapshot_and_scope(c: &mut Criterion) {
    c.bench_function("log_context_snapshot", |b| {
        b.iter(nebula_log::observability::current_contexts);
    });

    c.bench_function("log_context_scope_sync_overhead", |b| {
        b.iter(|| {
            let ctx = nebula_log::Context::new()
                .with_request_id("req-bench")
                .with_user_id("user-bench")
                .with_field("shard", 7);
            ctx.scope_sync(|| black_box(1_u64 + 1));
        });
    });
}

fn bench_timing_utilities(c: &mut Criterion) {
    c.bench_function("log_timer_guard_drop", |b| {
        b.iter(|| {
            let _timer = nebula_log::TimerGuard::new("bench_timer_guard");
            black_box(42_u64);
        });
    });

    c.bench_function("log_timer_threshold_no_emit", |b| {
        b.iter(|| {
            let timer =
                nebula_log::Timer::new("bench_timer_threshold").threshold(Duration::from_secs(60));
            black_box(7_u64 * 6);
            black_box(timer.complete());
        });
    });
}

criterion_group!(
    log_hot_path,
    bench_emit_no_hooks,
    bench_emit_with_inline_noop_hook,
    bench_emit_with_bounded_counting_hook,
    bench_context_snapshot_and_scope,
    bench_timing_utilities
);
criterion_main!(log_hot_path);
