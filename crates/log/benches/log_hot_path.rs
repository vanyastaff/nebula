//! Steady-state hot-path benchmarks for nebula-log observability APIs.
//!
//! These benches focus on the operations that are most likely to regress in CI:
//! hook fan-out during `emit_event()`, projection of event payloads, and lookup
//! of merged logger resources from active execution/node contexts.

use std::{hint::black_box, sync::Arc, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_log::observability::{
    EventFields, ExecutionContext, HookPolicy, LoggerResource, NodeContext, ObservabilityEvent,
    ObservabilityFieldValue, ObservabilityFieldVisitor, ObservabilityHook, emit_event,
    event_data_json, get_current_logger_resource, register_hook, set_hook_policy, shutdown_hooks,
};

// === CONSTANTS & FIXTURES ===
// Hook counts reflect realistic deployment shapes:
// 0 = baseline registry fast path, 1 = single sink, 4 = common prod stack
// (logs + metrics + telemetry + notifications), 16 = plugin-rich stress case.
const EMIT_HOOK_COUNTS: [usize; 4] = [0, 1, 4, 16];

// Field counts represent a control payload, a typical lifecycle event, and a
// richer workflow event with identifiers, counters, and status metadata.
const PAYLOAD_FIELD_COUNTS: [usize; 3] = [0, 4, 12];

// Tag counts mirror sparse, typical, and dense contextual metadata overlays.
const RESOURCE_TAG_COUNTS: [usize; 3] = [0, 4, 16];

const TYPICAL_EVENT_FIELD_COUNT: usize = 4;
const HOOK_BUDGET_MS: u64 = 5;
const HOOK_QUEUE_CAPACITY: usize = 256;
const WARM_UP_TIME: Duration = Duration::from_millis(500);
const MEASUREMENT_TIME: Duration = Duration::from_secs(3);
const SAMPLE_SIZE: usize = 200;

#[derive(Debug, Clone, Copy)]
struct BenchEvent {
    field_count: usize,
}

impl BenchEvent {
    const fn new(field_count: usize) -> Self {
        Self { field_count }
    }
}

impl ObservabilityEvent for BenchEvent {
    fn name(&self) -> &str {
        "bench.operation"
    }

    fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor) {
        for index in 0..self.field_count.min(PAYLOAD_FIELD_COUNTS[2]) {
            match index {
                0 => visitor.record("operation", ObservabilityFieldValue::Str("http.request")),
                1 => visitor.record("context", ObservabilityFieldValue::Str("workflow.execute")),
                2 => visitor.record("node_id", ObservabilityFieldValue::Str("node-42")),
                3 => visitor.record("tenant_id", ObservabilityFieldValue::Str("tenant-a")),
                4 => visitor.record("attempt", ObservabilityFieldValue::U64(3)),
                5 => visitor.record("success", ObservabilityFieldValue::Bool(false)),
                6 => visitor.record("duration_ms", ObservabilityFieldValue::U64(87)),
                7 => visitor.record("queue_fill", ObservabilityFieldValue::F64(0.82)),
                8 => visitor.record("retryable", ObservabilityFieldValue::Bool(true)),
                9 => visitor.record("batch_size", ObservabilityFieldValue::U64(32)),
                10 => visitor.record("region", ObservabilityFieldValue::Str("eu-west-1")),
                11 => visitor.record("delta", ObservabilityFieldValue::I64(-2)),
                _ => unreachable!(),
            }
        }
    }
}

struct NoopHook;

impl ObservabilityHook for NoopHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {}
}

#[derive(Default)]
struct FieldScanVisitor {
    field_count: usize,
    approx_payload_bytes: usize,
}

impl ObservabilityFieldVisitor for FieldScanVisitor {
    fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>) {
        self.field_count += 1;
        self.approx_payload_bytes += key.len();

        match value {
            ObservabilityFieldValue::Str(value) => {
                self.approx_payload_bytes += value.len();
            },
            ObservabilityFieldValue::Bool(_) => {
                self.approx_payload_bytes += 1;
            },
            ObservabilityFieldValue::I64(_) | ObservabilityFieldValue::U64(_) => {
                self.approx_payload_bytes += 8;
            },
            ObservabilityFieldValue::F64(_) => {
                self.approx_payload_bytes += 8;
            },
        }
    }
}

struct FieldScanHook;

impl ObservabilityHook for FieldScanHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        let mut visitor = FieldScanVisitor::default();
        event.visit_fields(&mut visitor);
        black_box((visitor.field_count, visitor.approx_payload_bytes));
    }
}

#[derive(Debug, Clone, Copy)]
enum HookWorkload {
    Noop,
    FieldScan,
}

fn throughput_for(count: usize) -> Throughput {
    Throughput::Elements(u64::try_from(count.max(1)).unwrap_or(u64::MAX))
}

fn install_hooks(policy: HookPolicy, hook_count: usize, workload: HookWorkload) {
    shutdown_hooks();
    set_hook_policy(policy);

    for _ in 0..hook_count {
        match workload {
            HookWorkload::Noop => register_hook(Arc::new(NoopHook)),
            HookWorkload::FieldScan => register_hook(Arc::new(FieldScanHook)),
        }
    }
}

fn build_logger_resource(tag_count: usize, prefix: &str) -> LoggerResource {
    let mut resource = LoggerResource::new()
        .with_sentry_dsn(format!("https://{prefix}@example.invalid/project"))
        .with_sampling(0.25);

    for index in 0..tag_count {
        resource = resource.with_tag(format!("tag_{index}"), format!("{prefix}_{index}"));
    }

    resource
}

// === BENCHMARK FUNCTIONS ===

fn bench_emit_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("log/observability/emit_dispatch");
    group.warm_up_time(WARM_UP_TIME);
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    let event = BenchEvent::new(TYPICAL_EVENT_FIELD_COUNT);
    let event_ref: &dyn ObservabilityEvent = &event;

    for &hook_count in &EMIT_HOOK_COUNTS {
        group.throughput(Throughput::Elements(1));
        install_hooks(HookPolicy::Inline, hook_count, HookWorkload::Noop);
        group.bench_with_input(
            BenchmarkId::new("inline_noop", hook_count),
            &hook_count,
            |b, _| {
                b.iter(|| {
                    emit_event(black_box(event_ref));
                });
            },
        );
        shutdown_hooks();
    }

    for &hook_count in &EMIT_HOOK_COUNTS[1..] {
        group.throughput(Throughput::Elements(1));
        install_hooks(HookPolicy::Inline, hook_count, HookWorkload::FieldScan);
        group.bench_with_input(
            BenchmarkId::new("inline_field_scan", hook_count),
            &hook_count,
            |b, _| {
                b.iter(|| {
                    emit_event(black_box(event_ref));
                });
            },
        );
        shutdown_hooks();
    }

    for &hook_count in &EMIT_HOOK_COUNTS[1..] {
        group.throughput(Throughput::Elements(1));
        install_hooks(
            HookPolicy::Bounded {
                timeout_ms: HOOK_BUDGET_MS,
                queue_capacity: HOOK_QUEUE_CAPACITY,
            },
            hook_count,
            HookWorkload::FieldScan,
        );
        group.bench_with_input(
            BenchmarkId::new("bounded_field_scan", hook_count),
            &hook_count,
            |b, _| {
                b.iter(|| {
                    emit_event(black_box(event_ref));
                });
            },
        );
        shutdown_hooks();
    }

    group.finish();
    shutdown_hooks();
}

fn bench_payload_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("log/observability/payload_projection");
    group.warm_up_time(WARM_UP_TIME);
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    for &field_count in &PAYLOAD_FIELD_COUNTS {
        let event = BenchEvent::new(field_count);
        let event_ref: &dyn ObservabilityEvent = &event;

        group.throughput(throughput_for(field_count));
        group.bench_with_input(
            BenchmarkId::new("display", field_count),
            &field_count,
            |b, _| {
                b.iter(|| {
                    black_box(EventFields::new(black_box(event_ref)).to_string());
                });
            },
        );

        group.throughput(throughput_for(field_count));
        group.bench_with_input(
            BenchmarkId::new("json", field_count),
            &field_count,
            |b, _| {
                b.iter(|| {
                    black_box(event_data_json(black_box(event_ref)));
                });
            },
        );
    }

    group.finish();
}

#[expect(
    clippy::excessive_nesting,
    reason = "bench uses nested context scopes and Criterion closure layers"
)]
fn bench_logger_resource_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("log/observability/logger_resource_lookup");
    group.warm_up_time(WARM_UP_TIME);
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    for &tag_count in &RESOURCE_TAG_COUNTS {
        let execution_only = ExecutionContext::new("exec-1", "workflow-1", "tenant-1")
            .with_resource(build_logger_resource(tag_count, "execution"));

        group.throughput(throughput_for(tag_count));
        execution_only.scope_sync(|| {
            group.bench_with_input(
                BenchmarkId::new("execution_only", tag_count),
                &tag_count,
                |b, _| {
                    b.iter(|| {
                        black_box(get_current_logger_resource());
                    });
                },
            );
        });

        let execution = ExecutionContext::new("exec-2", "workflow-2", "tenant-1")
            .with_resource(build_logger_resource(tag_count, "execution"));
        let node = NodeContext::new("node-7", "http.request")
            .with_retry_count(2)
            .with_resource(build_logger_resource(tag_count, "node"));

        group.throughput(throughput_for(tag_count.saturating_mul(2)));
        execution.scope_sync(|| {
            node.scope_sync(|| {
                group.bench_with_input(
                    BenchmarkId::new("execution_plus_node", tag_count),
                    &tag_count,
                    |b, _| {
                        b.iter(|| {
                            black_box(get_current_logger_resource());
                        });
                    },
                );
            });
        });
    }

    group.finish();
}

// === HARNESS ===

criterion_group!(
    benches,
    bench_emit_dispatch,
    bench_payload_projection,
    bench_logger_resource_lookup
);
criterion_main!(benches);
